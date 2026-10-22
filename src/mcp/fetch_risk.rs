//! Whether a bot's WebFetch needs a card: anything that can reach this computer or the local
//! network. The URL is read the way a browser (WHATWG URL parser) reads it, so `2130706433`,
//! `0x7f000001`, `0177.0.0.1`, `127.1`, `%6cocalhost` and `localhost.` all count as what they
//! really are. A name that isn't an IP is resolved, and one that points at this computer or a
//! private network asks too (a name that doesn't resolve asks as well).
//!
//! The local dev server case stays free: `localhost`, `127.0.0.1` or `[::1]` on a port that
//! isn't kwnt's and a path outside `/api/` (see [`super::shell_risk::kwnt_url`]).
//!
//! Not a sandbox: the fetch resolves the name again, so a name whose answer changes between the
//! check and the fetch (DNS rebinding) isn't caught here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;

/// How long the check waits for a name to resolve before asking instead.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Where an address points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// This computer (loopback, or "any address").
    Local,
    /// A private, link-local or carrier-grade NAT network (routers, NAS, cloud metadata).
    Private,
    Public,
}

/// Why a WebFetch of `url` needs a card, or `None` when it may run without one.
pub fn fetch_risk(url: &str) -> Option<String> {
    fetch_risk_with(url, &system_resolve)
}

/// [`fetch_risk`] with the name lookup supplied (tests).
pub fn fetch_risk_with(url: &str, resolve: &dyn Fn(&str, u16) -> Option<Vec<IpAddr>>) -> Option<String> {
    let url = url.trim();
    let with_scheme = if url.contains("://") { url.to_string() } else { format!("http://{url}") };
    let Ok(parsed) = url::Url::parse(&with_scheme) else {
        return Some("kwnt can't read this URL".into());
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return Some(format!("it fetches a `{}:` URL", parsed.scheme()));
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    let local_and_kwnt = || {
        // kwnt's own port or its API path, from any local spelling (the dev server stays free)
        super::shell_risk::kwnt_url(&format!("http://127.0.0.1:{port}{}", parsed.path()))
            .then(|| "it fetches from kwnt itself, whose settings only the person may change".to_string())
    };
    match parsed.host() {
        None => Some("kwnt can't tell where this URL goes".into()),
        Some(url::Host::Ipv4(ip)) => by_reach(reach(IpAddr::V4(ip)), &ip.to_string(), local_and_kwnt),
        Some(url::Host::Ipv6(ip)) => by_reach(reach(IpAddr::V6(ip)), &ip.to_string(), local_and_kwnt),
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if name.is_empty() {
                return Some("kwnt can't tell where this URL goes".into());
            }
            if name == "localhost" || name.ends_with(".localhost") || name == "localhost.localdomain" {
                return local_and_kwnt();
            }
            let Some(addrs) = resolve(&name, port).filter(|a| !a.is_empty()) else {
                return Some(format!("kwnt couldn't look up `{name}`, so it can't tell whether it is on this computer"));
            };
            // A public name that points here: no dev server exemption, it's a way around the check
            match addrs.iter().map(|ip| reach(*ip)).find(|r| *r != Reach::Public) {
                Some(Reach::Local) => Some(format!("`{name}` points at this computer")),
                Some(_) => Some(format!("`{name}` points into a private network")),
                None => None,
            }
        }
    }
}

fn by_reach(reach: Reach, shown: &str, local_and_kwnt: impl FnOnce() -> Option<String>) -> Option<String> {
    match reach {
        Reach::Local => local_and_kwnt(),
        Reach::Private => Some(format!("{shown} is on a private or link-local network")),
        Reach::Public => None,
    }
}

fn reach(ip: IpAddr) -> Reach {
    match ip {
        IpAddr::V4(v4) => reach_v4(v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return Reach::Local;
            }
            if let Some(v4) = embedded_v4(v6) {
                return reach_v4(v4);
            }
            let first = v6.segments()[0];
            // fc00::/7 unique local, fe80::/10 link-local, fec0::/10 old site-local
            if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || (first & 0xffc0) == 0xfec0 {
                Reach::Private
            } else {
                Reach::Public
            }
        }
    }
}

fn reach_v4(ip: Ipv4Addr) -> Reach {
    let [a, b, ..] = ip.octets();
    if ip.is_loopback() || ip.is_unspecified() || a == 0 {
        Reach::Local
    } else if ip.is_private() || ip.is_link_local() || ip.is_broadcast() || (a == 100 && (64..128).contains(&b)) {
        Reach::Private
    } else {
        Reach::Public
    }
}

/// The IPv4 address inside an IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`)
/// or NAT64 (`64:ff9b::a.b.c.d`) address.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return Some(v4);
    }
    let s = ip.segments();
    let tail = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
    match s {
        [0, 0, 0, 0, 0, 0, _, _] => Some(tail),
        [0x64, 0xff9b, 0, 0, 0, 0, _, _] => Some(tail),
        _ => None,
    }
}

/// The system's answer for `name`, or `None` when it fails or takes longer than
/// [`RESOLVE_TIMEOUT`].
fn system_resolve(name: &str, port: u16) -> Option<Vec<IpAddr>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let target = format!("{name}:{port}");
    std::thread::spawn(move || {
        let found = target.to_socket_addrs().map(|a| a.map(|s| s.ip()).collect::<Vec<_>>()).ok();
        let _ = tx.send(found);
    });
    rx.recv_timeout(RESOLVE_TIMEOUT).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake DNS: `localtest.me` and `*.nip.io` point at 127.0.0.1, `nas.example` at
    /// 192.168.1.20, `example.com` at a public address, anything else doesn't resolve.
    fn fake(name: &str, _port: u16) -> Option<Vec<IpAddr>> {
        match name {
            "localtest.me" => Some(vec!["127.0.0.1".parse().unwrap()]),
            n if n.ends_with(".nip.io") => Some(vec!["127.0.0.1".parse().unwrap()]),
            "nas.example" => Some(vec!["192.168.1.20".parse().unwrap()]),
            "dual.example" => Some(vec!["93.184.216.34".parse().unwrap(), "::1".parse().unwrap()]),
            "example.com" | "docs.rs" => Some(vec!["93.184.216.34".parse().unwrap()]),
            _ => None,
        }
    }

    fn asks(url: &str) -> bool {
        fetch_risk_with(url, &fake).is_some()
    }

    /// Rigel's S1 list (scripts-rigel/probe4-webfetch.log), with kwnt on 8799.
    #[test]
    fn every_local_or_private_spelling_asks() {
        super::super::shell_risk::set_serve_port(8799);
        for url in [
            "http://127.1:8799/",
            "http://2130706433:8799/",
            "http://2130706433:8799/api/passwords/store",
            "http://0x7f000001:8799/",
            "http://0177.0.0.1:8799/",
            "http://0x7f.1:8799/",
            "http://localhost.:8799/",
            "http://[::ffff:7f00:1]:8799/",
            "http://[::ffff:127.0.0.1]:8799/api/x",
            "http://127.0.0.1.nip.io:8799/api/passwords/store",
            "http://localtest.me:8799/api/x",
            "http://localtest.me:3000/",
            "http://foo.localhost:8799/api/x",
            "http://localhost.localdomain:8799/",
            "http://0:8799/",
            "http://0.0.0.0:8799/",
            "http://127.0.0.1:8799",
            "http://127.5.5.5:8799/",
            "http://[0:0:0:0:0:0:0:1]:8799/",
            "http://LOCALHOST:8799/",
            "http://%6cocalhost:8799/api/x",
            "http://localhost%2e:8799/",
            "http://localhost:8799%2f",
            "http://user@localhost:8799",
            "http://example.com@127.0.0.1:8799/",
            "http://localhost:3000/api/spaces",
            "http://localhost:3000/%61pi/x",
            "localhost:8799",
            "http://169.254.169.254/latest/meta-data/",
            "http://metadata.google.internal/",
            "http://10.0.0.1:8799/",
            "http://192.168.1.1/",
            "http://172.16.5.4/",
            "http://100.64.0.1/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::127.0.0.1]:8799/",
            "http://[64:ff9b::a00:1]/",
            "http://my-nas.local/",
            "http://nas.example/",
            "http://dual.example/",
            "ftp://example.com/",
            "file:///etc/passwd",
            "http://",
        ] {
            assert!(asks(url), "{url} should ask");
        }
    }

    #[test]
    fn public_hosts_and_the_local_dev_server_stay_free() {
        super::super::shell_risk::set_serve_port(8799);
        for url in [
            "https://example.com/",
            "https://example.com/api/x",
            "http://127.0.0.1%23@example.com:8799/",
            "https://docs.rs/url",
            "http://93.184.216.34/",
            "http://[2606:2800:220:1:248:1893:25c8:1946]/",
            "http://localhost:3000/",
            "http://127.0.0.1:3000/index.html",
            "http://[::1]:3000/",
            "http://127.0.0.1/",
            "https://localhost/",
        ] {
            assert_eq!(fetch_risk_with(url, &fake), None, "{url} should run");
        }
    }

    #[test]
    fn the_reason_names_what_was_risky() {
        super::super::shell_risk::set_serve_port(8799);
        assert!(fetch_risk_with("http://localtest.me/", &fake).unwrap().contains("points at this computer"));
        assert!(fetch_risk_with("http://10.1.2.3/", &fake).unwrap().contains("private"));
        assert!(fetch_risk_with("http://nowhere.invalid/", &fake).unwrap().contains("couldn't look up"));
        assert!(fetch_risk_with("http://2130706433:8799/", &fake).unwrap().contains("kwnt itself"));
    }
}
