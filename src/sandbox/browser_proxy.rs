//! Sending the bots' browser out through a proxy, so sites see a home connection instead of a
//! datacenter one.
//!
//! `KWNT_BROWSER_PROXY` holds the proxy (`socks5://host:port`, `socks5h://…`, `http://host:port`
//! or `https://host:port`) and `KWNT_BROWSER_PROXY_BYPASS` adds hosts that skip it (`;` or `,`
//! separated, Chromium's bypass syntax). Kwant's own services are always bypassed.
//!
//! Credentials are refused: Chromium cannot sign in to a SOCKS proxy at all, and a password in a
//! command line, container environment or log would leak. Use a proxy that trusts the network it
//! is on (a loopback relay, a WireGuard helper). Nothing here ever echoes the value it was given,
//! so a mistyped secret does not end up in an error message or the log.

use std::time::Duration;

pub const ENV_PROXY: &str = "KWNT_BROWSER_PROXY";
pub const ENV_BYPASS: &str = "KWNT_BROWSER_PROXY_BYPASS";

/// Hosts that never go through the proxy: Kwant's own compose services, loopback and private
/// ranges. `docker/browser-entrypoint.sh` carries the same list (a test keeps them equal).
pub const DEFAULT_BYPASS: &str =
    "laya;kwnt;browser;localhost;127.0.0.0/8;10.0.0.0/8;172.16.0.0/12;192.168.0.0/16;169.254.0.0/16";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserProxy {
    /// `scheme://host:port`, without credentials, path or query.
    pub url: String,
    /// Extra bypass entries (after [`DEFAULT_BYPASS`]).
    pub bypass: Vec<String>,
}

impl BrowserProxy {
    /// Reads the proxy from the environment. `Ok(None)`: not set. `Err`: set but unusable; the
    /// message never contains the value.
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::parse(
            std::env::var(ENV_PROXY).ok().as_deref().unwrap_or(""),
            std::env::var(ENV_BYPASS).ok().as_deref().unwrap_or(""),
        )
    }

    pub fn parse(raw: &str, bypass: &str) -> Result<Option<Self>, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let bad = |why: &str| Err(format!("{ENV_PROXY} is not usable: {why}"));
        let Some((scheme, rest)) = raw.split_once("://") else {
            return bad("expected socks5://host:port or http://host:port");
        };
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "socks5" | "socks5h" | "http" | "https") {
            return bad("the scheme must be socks5, socks5h, http or https");
        }
        if rest.contains('@') {
            return bad("credentials are not supported (Chromium cannot sign in to SOCKS, and a password would leak); use a proxy that trusts this network");
        }
        let rest = rest.trim_end_matches('/');
        if rest.is_empty() || rest.contains(['/', '?', '#', ' ', ';', ',', '"', '\'']) {
            return bad("expected only host:port after the scheme");
        }
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(':') || h.starts_with('[') => (h, p),
            _ => return bad("a port is required"),
        };
        if host.is_empty() || (host.contains(':') && !host.starts_with('[')) || host.contains(['\\', '*']) || port.parse::<u16>().ok().filter(|p| *p > 0).is_none() {
            return bad("expected a host and a port from 1 to 65535");
        }
        let mut extra = Vec::new();
        for entry in bypass.split([';', ',']).map(str::trim).filter(|e| !e.is_empty()) {
            if entry.contains(['"', '\'', ' ', '\\']) {
                return Err(format!("{ENV_BYPASS} is not usable: an entry has a space or quote"));
            }
            extra.push(entry.to_string());
        }
        Ok(Some(Self { url: format!("{scheme}://{host}:{port}"), bypass: extra }))
    }

    /// The whole bypass list, Chromium syntax.
    pub fn bypass_list(&self) -> String {
        let mut list = DEFAULT_BYPASS.to_string();
        for entry in &self.bypass {
            list.push(';');
            list.push_str(entry);
        }
        list
    }

    /// Chromium's switches for this proxy. `socks5` makes Chromium resolve names through the
    /// proxy, so site names do not leak to the local resolver.
    pub fn chromium_args(&self) -> Vec<String> {
        vec![format!("--proxy-server={}", self.url), format!("--proxy-bypass-list={}", self.bypass_list())]
    }

    pub fn host_port(&self) -> (String, u16) {
        let rest = self.url.split_once("://").map_or(self.url.as_str(), |(_, r)| r);
        let (host, port) = rest.rsplit_once(':').unwrap_or((rest, "0"));
        (host.trim_matches(['[', ']']).to_string(), port.parse().unwrap_or(0))
    }

    pub fn is_socks(&self) -> bool {
        self.url.starts_with("socks5")
    }
}

/// The switches for the Chromium kwant starts itself (local mode), or none without a proxy. A
/// bad value is logged (without the value) and ignored rather than blocking the browser.
pub fn local_chrome_args() -> Vec<String> {
    match BrowserProxy::from_env() {
        Ok(Some(p)) => p.chromium_args(),
        Ok(None) => Vec::new(),
        Err(why) => {
            tracing::warn!("browser proxy ignored: {why}");
            Vec::new()
        }
    }
}

/// What the doctor learned about the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyFacts {
    pub url: String,
    pub reachable: bool,
    pub proxied_ip: Option<String>,
    pub direct_ip: Option<String>,
}

const IP_HOST: &str = "api.ipify.org";

/// Pulls the address out of an HTTP response from the IP echo service.
fn ip_from_response(response: &str) -> Option<String> {
    let body = response.split("\r\n\r\n").nth(1)?.trim();
    body.parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
}

/// `GET http://api.ipify.org` through a SOCKS5 proxy (no authentication), with the name resolved
/// by the proxy. A minimal client, so kwant needs no extra SOCKS dependency.
async fn exit_ip_via_socks(host: &str, port: u16) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let work = async {
        let mut s = tokio::net::TcpStream::connect((host, port)).await.ok()?;
        s.write_all(&[5, 1, 0]).await.ok()?;
        let mut reply = [0u8; 2];
        s.read_exact(&mut reply).await.ok()?;
        if reply != [5, 0] {
            return None;
        }
        let mut req = vec![5, 1, 0, 3, IP_HOST.len() as u8];
        req.extend_from_slice(IP_HOST.as_bytes());
        req.extend_from_slice(&80u16.to_be_bytes());
        s.write_all(&req).await.ok()?;
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.ok()?;
        if head[1] != 0 {
            return None;
        }
        let skip = match head[3] {
            1 => 4 + 2,
            4 => 16 + 2,
            3 => {
                let mut n = [0u8; 1];
                s.read_exact(&mut n).await.ok()?;
                n[0] as usize + 2
            }
            _ => return None,
        };
        let mut rest = vec![0u8; skip];
        s.read_exact(&mut rest).await.ok()?;
        s.write_all(format!("GET / HTTP/1.1\r\nHost: {IP_HOST}\r\nConnection: close\r\nUser-Agent: kwnt-doctor\r\n\r\n").as_bytes())
            .await
            .ok()?;
        let mut out = Vec::new();
        s.take(4096).read_to_end(&mut out).await.ok()?;
        ip_from_response(&String::from_utf8_lossy(&out))
    };
    tokio::time::timeout(Duration::from_secs(8), work).await.ok().flatten()
}

/// Looks at the proxy from where kwant runs: is it reachable, and does a request through it leave
/// from another address than a direct one. Never includes anything but the credential-free URL.
pub async fn probe(proxy: &BrowserProxy) -> ProxyFacts {
    let (host, port) = proxy.host_port();
    let reachable = matches!(
        tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect((host.as_str(), port))).await,
        Ok(Ok(_))
    );
    let http_ip = |client: reqwest::Client| async move {
        let text = client.get(format!("http://{IP_HOST}/")).send().await.ok()?.text().await.ok()?;
        text.trim().parse::<std::net::IpAddr>().ok().map(|ip| ip.to_string())
    };
    let timeout = Duration::from_secs(8);
    let direct = reqwest::Client::builder().timeout(timeout).no_proxy().build().ok();
    let direct_ip = match direct {
        Some(c) => http_ip(c).await,
        None => None,
    };
    let proxied_ip = if !reachable {
        None
    } else if proxy.is_socks() {
        exit_ip_via_socks(&host, port).await
    } else {
        match reqwest::Proxy::all(&proxy.url).ok().and_then(|p| reqwest::Client::builder().timeout(timeout).proxy(p).build().ok()) {
            Some(c) => http_ip(c).await,
            None => None,
        }
    };
    ProxyFacts { url: proxy.url.clone(), reachable, proxied_ip, direct_ip }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> BrowserProxy {
        BrowserProxy::parse(raw, "").unwrap().unwrap()
    }

    #[test]
    fn unset_or_blank_means_no_proxy() {
        assert_eq!(BrowserProxy::parse("", "").unwrap(), None);
        assert_eq!(BrowserProxy::parse("  ", "x.com").unwrap(), None);
    }

    #[test]
    fn the_four_schemes_are_accepted_and_normalised() {
        assert_eq!(ok("socks5://172.17.0.1:1080").url, "socks5://172.17.0.1:1080");
        assert_eq!(ok(" SOCKS5H://proxy.lan:9050/ ").url, "socks5h://proxy.lan:9050");
        assert_eq!(ok("http://p:3128").url, "http://p:3128");
        assert_eq!(ok("https://p:443").url, "https://p:443");
        assert_eq!(ok("socks5://[::1]:1080").host_port(), ("::1".to_string(), 1080));
    }

    #[test]
    fn a_bad_value_is_refused_without_repeating_it() {
        for raw in [
            "proxy:1080",
            "ftp://p:1",
            "socks5://p",
            "socks5://p:0",
            "socks5://p:99999",
            "socks5://:1080",
            "http://p:80/path",
            "http://p:80 --no-sandbox",
            "http://user:hunter2@p:80",
            "socks5://user:hunter2@p:1080",
        ] {
            let err = BrowserProxy::parse(raw, "").unwrap_err();
            assert!(err.contains(ENV_PROXY), "{err}");
            assert!(!err.contains("hunter2") && !err.contains("--no-sandbox") && !err.contains(raw), "{err}");
        }
    }

    #[test]
    fn credentials_are_refused_for_every_scheme() {
        for scheme in ["socks5", "socks5h", "http", "https"] {
            assert!(BrowserProxy::parse(&format!("{scheme}://a:b@h:1"), "").is_err());
            assert!(BrowserProxy::parse(&format!("{scheme}://a@h:1"), "").is_err());
        }
    }

    #[test]
    fn kwants_own_services_always_bypass_the_proxy_and_extras_follow() {
        let p = BrowserProxy::parse("socks5://h:1", "example.com, *.corp;;10.9.0.0/16").unwrap().unwrap();
        let list = p.bypass_list();
        assert!(list.starts_with(DEFAULT_BYPASS), "{list}");
        assert!(list.ends_with(";example.com;*.corp;10.9.0.0/16"), "{list}");
        for service in ["laya", "kwnt", "browser", "localhost", "172.16.0.0/12"] {
            assert!(DEFAULT_BYPASS.split(';').any(|e| e == service), "{service}");
        }
        assert_eq!(
            p.chromium_args(),
            vec!["--proxy-server=socks5://h:1".to_string(), format!("--proxy-bypass-list={list}")]
        );
    }

    #[test]
    fn a_bypass_entry_cannot_smuggle_a_switch() {
        assert!(BrowserProxy::parse("socks5://h:1", "a.com --no-sandbox").is_err());
        assert!(BrowserProxy::parse("socks5://h:1", "a\"b").is_err());
    }

    #[test]
    fn the_container_entrypoint_uses_the_same_default_bypass() {
        let script = include_str!("../../docker/browser-entrypoint.sh");
        assert!(script.contains(DEFAULT_BYPASS), "docker/browser-entrypoint.sh default bypass drifted");
    }

    #[test]
    fn the_exit_address_is_read_from_the_echo_response() {
        assert_eq!(ip_from_response("HTTP/1.1 200 OK\r\nA: b\r\n\r\n203.0.113.7").as_deref(), Some("203.0.113.7"));
        assert_eq!(ip_from_response("HTTP/1.1 200 OK\r\n\r\n2001:db8::1\n").as_deref(), Some("2001:db8::1"));
        assert_eq!(ip_from_response("HTTP/1.1 403 Forbidden\r\n\r\n<html>nope</html>"), None);
        assert_eq!(ip_from_response("garbage"), None);
    }

    #[tokio::test]
    async fn a_closed_port_is_reported_unreachable_with_no_addresses() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let facts = probe(&ok(&format!("socks5://127.0.0.1:{port}"))).await;
        assert!(!facts.reachable);
        assert_eq!(facts.proxied_ip, None);
    }

    #[tokio::test]
    async fn the_socks_probe_does_the_handshake_and_reads_the_address() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 3];
            s.read_exact(&mut hello).await.unwrap();
            assert_eq!(hello, [5, 1, 0]);
            s.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 5];
            s.read_exact(&mut head).await.unwrap();
            assert_eq!(&head[..4], &[5, 1, 0, 3]);
            let mut name = vec![0u8; head[4] as usize + 2];
            s.read_exact(&mut name).await.unwrap();
            assert_eq!(&name[..name.len() - 2], IP_HOST.as_bytes());
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).await.unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("GET / HTTP/1.1"));
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\n198.51.100.9").await.unwrap();
        });
        assert_eq!(exit_ip_via_socks("127.0.0.1", port).await.as_deref(), Some("198.51.100.9"));
    }
}
