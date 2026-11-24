//! Where guests open a pocket (pa10a "stable address", pa6 "join from anywhere", plan section 7).
//! In order:
//!
//! 1. **Own https**: the `pocket_public_url` setting, an `https://` address the person already
//!    points at the guest port (their own domain or reverse proxy). Read only; kwant never
//!    changes it.
//! 2. **Tailscale Funnel**: `tailscale funnel status --json` (3 s at most, kept for 60 s) shows a
//!    funnel whose `/` proxies to the guest port. A probe only: kwant never turns Funnel on.
//! 3. **Quick tunnel** (pa6): a Cloudflare quick tunnel the person turned on for this pocket
//!    ([`super::tunnel`]). A new random address each start, so a kit that runs for weeks
//!    ([`super::kits::Kit::stable_address_only`], Secret Santa) is never offered one.
//! 4. **Same Wi-Fi**: `http://<this computer's LAN address>:<guest port>`, only when the guest
//!    listener is reachable from the network (it binds the main `host`, `127.0.0.1` by default).
//!
//! The answer (`join`, on the person's `GET /api/pockets/:code`) is
//! `{mode: "anywhere"|"wifi"|"none", anywhere_url, anywhere_kind: "https"|"funnel"|"quick"|null,
//!   wifi_url, wifi_note, quick_tunnel: bool, tunnel: {state: "off"|"starting"|"on"|"failed", url, note}}`.

use crate::server::AppState;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The setting with the person's own https address for the guest port.
pub const PUBLIC_URL_SETTING: &str = "pocket_public_url";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_KEEP: Duration = Duration::from_secs(60);
const TAILSCALE_PATHS: &[&str] = &["tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/usr/local/bin/tailscale", "/opt/homebrew/bin/tailscale"];

/// A stable `https://host[:port]` (no path, query or user info), or `None`.
pub fn clean_public_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_end_matches('/');
    let rest = t.strip_prefix("https://")?;
    let ok = !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)) && !rest.starts_with(['.', '-', ':']);
    ok.then(|| format!("https://{}", rest.to_ascii_lowercase()))
}

/// The funnel address that reaches `port` on this computer, from `tailscale funnel status --json`
/// (the same shape as `tailscale serve status --json`): a `Web` entry `host:port` whose `/`
/// handler proxies to `127.0.0.1:<port>` or `localhost:<port>`, and `AllowFunnel[host:port]`.
pub fn funnel_url(status: &Value, port: u16) -> Option<String> {
    let web = status.get("Web")?.as_object()?;
    let allow = status.get("AllowFunnel").and_then(Value::as_object);
    let mut found: Vec<String> = web
        .iter()
        .filter(|(hp, _)| allow.and_then(|a| a.get(*hp)).and_then(Value::as_bool) == Some(true))
        .filter(|(_, w)| {
            let proxy = w.pointer("/Handlers/~1/Proxy").and_then(Value::as_str).unwrap_or("");
            let target = proxy.trim_end_matches('/').trim_start_matches("http://");
            target == format!("127.0.0.1:{port}") || target == format!("localhost:{port}") || target == port.to_string()
        })
        .filter_map(|(hp, _)| {
            let (host, p) = hp.rsplit_once(':')?;
            if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)) {
                return None;
            }
            Some(if p == "443" { format!("https://{host}") } else { format!("https://{host}:{p}") })
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The Same Wi-Fi address, or why there is none. `bound` is where the guest listener listens.
pub fn wifi(bound: Option<SocketAddr>, lan_ip: Option<IpAddr>) -> (Option<String>, Option<&'static str>) {
    let Some(addr) = bound else {
        return (None, Some("listener_off"));
    };
    let ip = if addr.ip().is_unspecified() {
        match lan_ip {
            Some(ip) => ip,
            None => return (None, Some("no_network")),
        }
    } else if addr.ip().is_loopback() {
        return (None, Some("this_computer_only"));
    } else {
        addr.ip()
    };
    let host = match ip {
        IpAddr::V6(v6) => format!("[{v6}]"),
        IpAddr::V4(v4) => v4.to_string(),
    };
    (Some(format!("http://{host}:{}", addr.port())), None)
}

/// The anywhere address in order of preference: own https, then Funnel, then a quick tunnel.
/// `quick_tunnel` and `tunnel` are added by [`join`].
pub fn choose(code: &str, own: Option<&str>, funnel: Option<&str>, quick: Option<&str>, wifi_base: Option<&str>, wifi_note: Option<&str>) -> Value {
    let (anywhere, kind) = match (own, funnel, quick) {
        (Some(u), _, _) => (Some(u), Some("https")),
        (None, Some(u), _) => (Some(u), Some("funnel")),
        (None, None, Some(u)) => (Some(u), Some("quick")),
        _ => (None, None),
    };
    let link = |base: &str| format!("{base}/p/{code}");
    json!({
        "mode": if anywhere.is_some() { "anywhere" } else if wifi_base.is_some() { "wifi" } else { "none" },
        "anywhere_url": anywhere.map(link),
        "anywhere_kind": kind,
        "wifi_url": wifi_base.map(link),
        "wifi_note": wifi_note,
    })
}

/// This computer's address on the local network: the source address the system would use to
/// reach a public address (a UDP "connect" sends nothing).
fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:80").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

static FUNNEL: Mutex<Option<(Instant, u16, Option<String>)>> = Mutex::new(None);

async fn probe_funnel(port: u16) -> Option<String> {
    if let Some((at, p, url)) = FUNNEL.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if *p == port && at.elapsed() < PROBE_KEEP {
            return url.clone();
        }
    }
    let mut url = None;
    for bin in TAILSCALE_PATHS {
        let run = tokio::process::Command::new(bin).args(["funnel", "status", "--json"]).kill_on_drop(true).output();
        match tokio::time::timeout(PROBE_TIMEOUT, run).await {
            Ok(Ok(out)) if out.status.success() => {
                url = serde_json::from_slice::<Value>(&out.stdout).ok().and_then(|v| funnel_url(&v, port));
                break;
            }
            Ok(Ok(_)) => break,
            Ok(Err(_)) => continue,
            Err(_) => break,
        }
    }
    *FUNNEL.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), port, url.clone()));
    url
}

/// The `join` block for one pocket.
pub async fn join(state: &AppState, s: &super::Snapshot) -> Value {
    let code = s.row.code.as_str();
    let offered = super::tunnel::refusal(s).is_none();
    let quick = if offered { state.pocket.tunnel.url_for(code) } else { None };
    let tunnel = if offered { state.pocket.tunnel.view(code) } else { json!({ "state": "off", "url": null, "note": null }) };
    let own = state.db.get_setting(PUBLIC_URL_SETTING).await.ok().flatten().and_then(|u| clean_public_url(&u));
    let funnel = match (own.is_none(), super::pocket_port(&state.config)) {
        (true, Some(port)) => probe_funnel(port).await,
        _ => None,
    };
    let (wifi_base, note) = wifi(state.pocket.listener.addr(), tokio::task::spawn_blocking(lan_ip).await.ok().flatten());
    let mut j = choose(code, own.as_deref(), funnel.as_deref(), quick.as_deref(), wifi_base.as_deref(), note);
    j["quick_tunnel"] = json!(offered);
    j["tunnel"] = tunnel;
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_https_only() {
        assert_eq!(clean_public_url(" https://Santa.Example.com/ ").as_deref(), Some("https://santa.example.com"));
        assert_eq!(clean_public_url("https://box.example.com:8443").as_deref(), Some("https://box.example.com:8443"));
        for bad in ["http://x.com", "https://", "https://x.com/path", "https://user@x.com", "https://x.com?a=1", "ftp://x", "https://.x"] {
            assert_eq!(clean_public_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn funnel_status_is_read_not_changed() {
        let status = json!({
            "TCP": { "443": { "HTTPS": true } },
            "Web": {
                "mac.tail1234.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:8788" } } },
                "mac.tail1234.ts.net:8443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:9999" } } }
            },
            "AllowFunnel": { "mac.tail1234.ts.net:443": true, "mac.tail1234.ts.net:8443": true }
        });
        assert_eq!(funnel_url(&status, 8788).as_deref(), Some("https://mac.tail1234.ts.net"));
        assert_eq!(funnel_url(&status, 9999).as_deref(), Some("https://mac.tail1234.ts.net:8443"));
        assert_eq!(funnel_url(&status, 1234), None, "a funnel to another port isn't ours");
        // Served on the tailnet but not funnelled: not "anywhere"
        let tailnet_only = json!({ "Web": { "mac.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:8788" } } } }, "AllowFunnel": {} });
        assert_eq!(funnel_url(&tailnet_only, 8788), None);
        // A path handler isn't the guest page's root
        let path = json!({ "Web": { "mac.ts.net:443": { "Handlers": { "/santa": { "Proxy": "http://127.0.0.1:8788" } } } }, "AllowFunnel": { "mac.ts.net:443": true } });
        assert_eq!(funnel_url(&path, 8788), None);
        assert_eq!(funnel_url(&json!({}), 8788), None);
        assert_eq!(funnel_url(&json!("garbage"), 8788), None);
    }

    #[test]
    fn same_wifi_needs_a_reachable_listener() {
        let lan: IpAddr = "192.168.1.20".parse().unwrap();
        assert_eq!(wifi(Some("0.0.0.0:8788".parse().unwrap()), Some(lan)), (Some("http://192.168.1.20:8788".into()), None));
        assert_eq!(wifi(Some("127.0.0.1:8788".parse().unwrap()), Some(lan)), (None, Some("this_computer_only")));
        assert_eq!(wifi(Some("10.0.0.5:8788".parse().unwrap()), Some(lan)), (Some("http://10.0.0.5:8788".into()), None));
        assert_eq!(wifi(Some("0.0.0.0:8788".parse().unwrap()), None), (None, Some("no_network")));
        assert_eq!(wifi(None, Some(lan)), (None, Some("listener_off")));
    }

    #[test]
    fn own_https_beats_funnel_beats_a_quick_tunnel_beats_wifi() {
        let quick = Some("https://bowl-cited-than-amplifier.trycloudflare.com");
        let j = choose("abcdef", Some("https://santa.example.com"), Some("https://mac.ts.net"), quick, Some("http://192.168.1.20:8788"), None);
        assert_eq!((j["mode"].as_str(), j["anywhere_kind"].as_str()), (Some("anywhere"), Some("https")));
        assert_eq!(j["anywhere_url"], "https://santa.example.com/p/abcdef");
        assert_eq!(j["wifi_url"], "http://192.168.1.20:8788/p/abcdef");
        let j = choose("abcdef", None, Some("https://mac.ts.net"), quick, None, Some("this_computer_only"));
        assert_eq!((j["anywhere_kind"].as_str(), j["anywhere_url"].as_str()), (Some("funnel"), Some("https://mac.ts.net/p/abcdef")));
        let j = choose("abcdef", None, None, quick, Some("http://192.168.1.20:8788"), None);
        assert_eq!((j["mode"].as_str(), j["anywhere_kind"].as_str()), (Some("anywhere"), Some("quick")));
        assert_eq!(j["anywhere_url"], "https://bowl-cited-than-amplifier.trycloudflare.com/p/abcdef");
        assert_eq!(j["wifi_url"], "http://192.168.1.20:8788/p/abcdef", "Same Wi-Fi is still offered beside it");
        let j = choose("abcdef", None, None, None, Some("http://192.168.1.20:8788"), None);
        assert_eq!((j["mode"].as_str(), j["anywhere_url"].is_null(), j["anywhere_kind"].is_null()), (Some("wifi"), true, true));
        let j = choose("abcdef", None, None, None, None, Some("this_computer_only"));
        assert_eq!((j["mode"].as_str(), j["wifi_note"].as_str()), (Some("none"), Some("this_computer_only")));
    }
}
