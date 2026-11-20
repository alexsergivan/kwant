//! A WireGuard tunnel for the bots' browser, without a host relay, iptables or root.
//!
//! The person pastes a WireGuard config (a router's "single device" connection, a VPN
//! provider's file). Kwant checks it, turns it into a `wireproxy` config (WireGuard in user
//! space, offering a SOCKS5 port) and keeps it in `<data dir>/wireguard/wireproxy.conf`, readable
//! by the owner only. The compose profile `wireguard` runs the `wireproxy` helper on it, and
//! `KWNT_BROWSER_PROXY=socks5://wireproxy:1080` points the browser at it ([`super::browser_proxy`]).
//!
//! The private key is a secret. It is never printed, logged, returned by the API, put in an
//! error message or written anywhere but that one file; every error here names the line's key
//! (`PrivateKey`), never its value.
//!
//! One WireGuard connection can only be used from one place at a time (a router hands the
//! tunnel to whoever connected last). Give kwant its own connection; never reuse one another
//! program already uses.

use base64::Engine as _;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Where the helper listens inside the compose network.
pub const HELPER_PROXY_URL: &str = "socks5://wireproxy:1080";
/// What `wireproxy` binds inside its container (the compose network only; never published).
pub const HELPER_BIND: &str = "0.0.0.0:1080";
pub const DIR: &str = "wireguard";
pub const FILE: &str = "wireproxy.conf";
/// A pasted config is a few hundred bytes; refuse anything absurd.
pub const MAX_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub public_key: String,
    pub preshared_key: Option<String>,
    pub endpoint: String,
    pub allowed_ips: Vec<String>,
    pub keepalive: u16,
}

/// A checked config. Holds the private key: it has no `Debug`/`Serialize`; use [`WgSummary`].
#[derive(Clone, PartialEq, Eq)]
pub struct WgConfig {
    private_key: String,
    pub addresses: Vec<String>,
    pub dns: Vec<String>,
    pub mtu: Option<u16>,
    pub peers: Vec<Peer>,
}

impl std::fmt::Debug for WgConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgConfig").field("private_key", &"<hidden>").field("peers", &self.peers.len()).finish()
    }
}

/// What may be shown: no key of any kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WgSummary {
    pub address: Vec<String>,
    pub dns: Vec<String>,
    pub endpoint: String,
    pub peers: usize,
    pub full_tunnel: bool,
    pub preshared_key: bool,
    pub warnings: Vec<String>,
}

fn is_key(v: &str) -> bool {
    base64::engine::general_purpose::STANDARD.decode(v).is_ok_and(|b| b.len() == 32)
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(l, _)| l).trim()
}

fn list(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// `a.b.c.d/24` or `fd00::2/64` as the single address WireGuard-in-user-space wants.
fn single_address(v: &str) -> Result<String, ()> {
    let ip = v.split('/').next().unwrap_or("");
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(a)) => Ok(format!("{a}/32")),
        Ok(std::net::IpAddr::V6(a)) => Ok(format!("{a}/128")),
        Err(_) => Err(()),
    }
}

fn endpoint_ok(v: &str) -> bool {
    let Some((host, port)) = v.rsplit_once(':') else { return false };
    let host = host.trim_matches(['[', ']']);
    !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
        && port.parse::<u16>().ok().filter(|p| *p > 0).is_some()
}

fn cidr_ok(v: &str) -> bool {
    let Some((ip, bits)) = v.split_once('/') else { return v.parse::<std::net::IpAddr>().is_ok() };
    match (ip.parse::<std::net::IpAddr>(), bits.parse::<u8>()) {
        (Ok(std::net::IpAddr::V4(_)), Ok(b)) => b <= 32,
        (Ok(std::net::IpAddr::V6(_)), Ok(b)) => b <= 128,
        _ => false,
    }
}

impl WgConfig {
    /// Reads a wg-quick style config (or kwant's own `wireproxy.conf`). Settings kwant's
    /// user-space tunnel has no use for (`PostUp`, `Table`, `ListenPort`…) are ignored. Errors
    /// say what is wrong and never repeat a value.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err("That is too long for a WireGuard config".into());
        }
        #[derive(PartialEq)]
        enum Section {
            None,
            Interface,
            Peer,
            Other,
        }
        let mut section = Section::None;
        let mut private_key: Option<String> = None;
        let (mut addresses, mut dns, mut mtu) = (Vec::new(), Vec::new(), None);
        let mut peers: Vec<Peer> = Vec::new();
        let mut cur: Option<(Option<String>, Option<String>, Option<String>, Vec<String>, Option<u16>)> = None;
        let finish = |cur: &mut Option<(Option<String>, Option<String>, Option<String>, Vec<String>, Option<u16>)>,
                          peers: &mut Vec<Peer>|
         -> Result<(), String> {
            if let Some((public, preshared, endpoint, allowed, keepalive)) = cur.take() {
                let n = peers.len() + 1;
                let public_key = public.ok_or(format!("Peer {n} has no PublicKey"))?;
                let endpoint = endpoint.ok_or(format!("Peer {n} has no Endpoint (host:port of the WireGuard server)"))?;
                peers.push(Peer {
                    public_key,
                    preshared_key: preshared,
                    endpoint,
                    allowed_ips: if allowed.is_empty() { vec!["0.0.0.0/0".into(), "::/0".into()] } else { allowed },
                    keepalive: keepalive.unwrap_or(25),
                });
            }
            Ok(())
        };
        for raw in text.lines() {
            let line = strip_comment(raw);
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                finish(&mut cur, &mut peers)?;
                section = match name.trim().to_ascii_lowercase().as_str() {
                    "interface" => Section::Interface,
                    "peer" => {
                        cur = Some((None, None, None, Vec::new(), None));
                        Section::Peer
                    }
                    _ => Section::Other,
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err("A line is not 'Key = value' (is this a WireGuard config?)".into());
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            match (&section, key.as_str()) {
                (Section::Interface, "privatekey") => {
                    if !is_key(value) {
                        return Err("PrivateKey is not a WireGuard key (44 characters of base64)".into());
                    }
                    private_key = Some(value.to_string());
                }
                (Section::Interface, "address") => {
                    for a in list(value) {
                        addresses.push(single_address(&a).map_err(|()| "Address is not an IP address with a prefix, like 10.0.0.2/32".to_string())?);
                    }
                }
                (Section::Interface, "dns") => {
                    for d in list(value) {
                        // search domains are not addresses; wireproxy only wants the servers
                        if d.parse::<std::net::IpAddr>().is_ok() {
                            dns.push(d);
                        }
                    }
                }
                (Section::Interface, "mtu") => {
                    mtu = Some(value.parse::<u16>().ok().filter(|m| (576..=9000).contains(m)).ok_or("MTU must be a number from 576 to 9000")?);
                }
                (Section::Peer, k) => {
                    let c = cur.as_mut().expect("a peer section opened it");
                    match k {
                        "publickey" => {
                            if !is_key(value) {
                                return Err("A peer's PublicKey is not a WireGuard key (44 characters of base64)".into());
                            }
                            c.0 = Some(value.to_string());
                        }
                        "presharedkey" => {
                            if !is_key(value) {
                                return Err("A peer's PresharedKey is not a WireGuard key (44 characters of base64)".into());
                            }
                            c.1 = Some(value.to_string());
                        }
                        "endpoint" => {
                            if !endpoint_ok(value) {
                                return Err("Endpoint must be host:port, like home.example.net:51820".into());
                            }
                            c.2 = Some(value.to_string());
                        }
                        "allowedips" => {
                            let ips = list(value);
                            if ips.iter().any(|i| !cidr_ok(i)) {
                                return Err("AllowedIPs must be addresses with a prefix, like 0.0.0.0/0".into());
                            }
                            c.3 = ips;
                        }
                        "persistentkeepalive" => {
                            c.4 = Some(value.parse::<u16>().map_err(|_| "PersistentKeepalive must be a number of seconds")?);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        finish(&mut cur, &mut peers)?;
        let private_key = private_key.ok_or("The [Interface] has no PrivateKey")?;
        if addresses.is_empty() {
            return Err("The [Interface] has no Address".into());
        }
        if peers.is_empty() {
            return Err("There is no [Peer] section".into());
        }
        Ok(Self { private_key, addresses, dns, mtu, peers })
    }

    pub fn summary(&self) -> WgSummary {
        let full_tunnel = self.peers.iter().any(|p| p.allowed_ips.iter().any(|ip| ip == "0.0.0.0/0"));
        let mut warnings = Vec::new();
        if !full_tunnel {
            warnings.push("AllowedIPs has no 0.0.0.0/0: only traffic to those networks would leave through the tunnel, so websites would still see this server's address. On a FRITZ!Box, make the connection a full tunnel.".to_string());
        }
        if self.dns.is_empty() {
            warnings.push("No DNS server: site names are looked up through the tunnel's peer only if it answers DNS. Most home routers do (the DNS line of their config).".to_string());
        }
        WgSummary {
            address: self.addresses.clone(),
            dns: self.dns.clone(),
            endpoint: self.peers[0].endpoint.clone(),
            peers: self.peers.len(),
            full_tunnel,
            preshared_key: self.peers.iter().any(|p| p.preshared_key.is_some()),
            warnings,
        }
    }

    /// The config `wireproxy` runs: this tunnel plus a SOCKS5 listener on `bind`.
    pub fn to_wireproxy(&self, bind: &str) -> String {
        let mut out = String::from("# Written by kwnt. Holds a private key: keep it readable by its owner only.\n[Interface]\n");
        out.push_str(&format!("Address = {}\n", self.addresses.join(", ")));
        out.push_str(&format!("PrivateKey = {}\n", self.private_key));
        if !self.dns.is_empty() {
            out.push_str(&format!("DNS = {}\n", self.dns.join(", ")));
        }
        if let Some(m) = self.mtu {
            out.push_str(&format!("MTU = {m}\n"));
        }
        for p in &self.peers {
            out.push_str(&format!("\n[Peer]\nPublicKey = {}\n", p.public_key));
            if let Some(k) = &p.preshared_key {
                out.push_str(&format!("PresharedKey = {k}\n"));
            }
            out.push_str(&format!("Endpoint = {}\nAllowedIPs = {}\nPersistentKeepalive = {}\n", p.endpoint, p.allowed_ips.join(", "), p.keepalive));
        }
        out.push_str(&format!("\n[Socks5]\nBindAddress = {bind}\n"));
        out
    }
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

pub fn conf_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join(FILE)
}

/// Checks `text` and keeps it as the helper's config (owner-only, replaced in one step). Returns
/// what may be shown. Nothing is written when the config is not usable.
pub fn save(data_dir: &Path, text: &str) -> Result<WgSummary, String> {
    let cfg = WgConfig::parse(text)?;
    let d = dir(data_dir);
    std::fs::create_dir_all(&d).map_err(|e| format!("Could not create {}: {}", d.display(), e.kind()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = d.join(format!(".{FILE}.{}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(cfg.to_wireproxy(HELPER_BIND).as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, conf_path(data_dir))
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not save the WireGuard config: {}", e.kind())
    })?;
    Ok(cfg.summary())
}

/// What is saved, without any key. `None`: nothing saved; `Err`: a file that is not usable.
pub fn summary(data_dir: &Path) -> Option<Result<WgSummary, String>> {
    let text = std::fs::read_to_string(conf_path(data_dir)).ok()?;
    Some(WgConfig::parse(&text).map(|c| c.summary()))
}

/// Deletes the saved config. Whether there was one.
pub fn remove(data_dir: &Path) -> bool {
    std::fs::remove_file(conf_path(data_dir)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 32 zero bytes, 32 bytes of 0x01 and of 0x02, base64
    const PRIV: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PUB: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
    const PSK: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";

    fn sample() -> String {
        format!(
            "[Interface]\nPrivateKey = {PRIV} # secret\nAddress = 192.168.178.201/24, fd00::2/64\nDNS = 192.168.178.1, fritz.box\nMTU = 1380\nPostUp = iptables -A x\nListenPort = 5000\n\n[Peer]\nPublicKey = {PUB}\nPresharedKey = {PSK}\nEndpoint = abc.myfritz.net:51820\nAllowedIPs = 0.0.0.0/0, ::/0\n"
        )
    }

    #[test]
    fn a_router_config_is_read_and_made_user_space_friendly() {
        let c = WgConfig::parse(&sample()).unwrap();
        assert_eq!(c.addresses, vec!["192.168.178.201/32", "fd00::2/128"]);
        assert_eq!(c.dns, vec!["192.168.178.1"]);
        assert_eq!(c.mtu, Some(1380));
        let out = c.to_wireproxy(HELPER_BIND);
        assert!(out.contains("Address = 192.168.178.201/32, fd00::2/128"), "{out}");
        assert!(out.contains("PersistentKeepalive = 25"), "{out}");
        assert!(out.contains("[Socks5]\nBindAddress = 0.0.0.0:1080"), "{out}");
        assert!(!out.contains("iptables") && !out.contains("ListenPort") && !out.contains("fritz.box"), "{out}");
        // what wireproxy writes is what kwant reads back
        assert_eq!(WgConfig::parse(&out).unwrap().summary(), c.summary());
    }

    #[test]
    fn the_summary_and_debug_never_carry_a_key() {
        let c = WgConfig::parse(&sample()).unwrap();
        for text in [format!("{c:?}"), serde_json::to_string(&c.summary()).unwrap(), format!("{:?}", c.summary())] {
            assert!(!text.contains(PRIV) && !text.contains(PUB) && !text.contains(PSK), "{text}");
        }
        let s = c.summary();
        assert_eq!(s.endpoint, "abc.myfritz.net:51820");
        assert!(s.full_tunnel && s.preshared_key && s.warnings.is_empty());
    }

    #[test]
    fn a_partial_tunnel_warns_that_sites_still_see_this_server() {
        let text = sample().replace("0.0.0.0/0, ::/0", "192.168.178.0/24");
        let s = WgConfig::parse(&text).unwrap().summary();
        assert!(!s.full_tunnel);
        assert!(s.warnings.iter().any(|w| w.contains("0.0.0.0/0")), "{:?}", s.warnings);
    }

    #[test]
    fn bad_configs_are_refused_with_a_reason_and_without_the_key() {
        let wrong_key = sample().replace(PRIV, "notakey");
        let cases = [
            ("", "no PrivateKey"),
            ("just words", "not 'Key = value'"),
            (wrong_key.as_str(), "PrivateKey is not a WireGuard key"),
            (&format!("[Interface]\nPrivateKey = {PRIV}\n[Peer]\nPublicKey = {PUB}\nEndpoint = h:1\n"), "no Address"),
            (&format!("[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32\n"), "no [Peer]"),
            (&format!("[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32\n[Peer]\nPublicKey = {PUB}\n"), "no Endpoint"),
            (&format!("[Interface]\nPrivateKey = {PRIV}\nAddress = 10.0.0.2/32\n[Peer]\nPublicKey = {PUB}\nEndpoint = nohostport\n"), "host:port"),
            (&sample().replace("0.0.0.0/0, ::/0", "everything"), "AllowedIPs"),
        ];
        for (text, want) in cases {
            let err = WgConfig::parse(text).unwrap_err();
            assert!(err.contains(want), "{err} (wanted {want})");
            assert!(!err.contains(PRIV) && !err.contains("notakey"), "{err}");
        }
        assert!(WgConfig::parse(&"x".repeat(MAX_CONFIG_BYTES + 1)).is_err());
    }

    #[test]
    fn saving_writes_one_owner_only_file_and_nothing_for_a_bad_config() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(save(tmp.path(), "[Interface]\nPrivateKey = bad\n").is_err());
        assert!(!conf_path(tmp.path()).exists());
        let s = save(tmp.path(), &sample()).unwrap();
        assert_eq!(s.peers, 1);
        let text = std::fs::read_to_string(conf_path(tmp.path())).unwrap();
        assert!(text.contains(PRIV) && text.contains("[Socks5]"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(conf_path(tmp.path())).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(dir(tmp.path())).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // no temp file is left behind, and saving again replaces it
        save(tmp.path(), &sample().replace("1380", "1400")).unwrap();
        assert_eq!(std::fs::read_dir(dir(tmp.path())).unwrap().count(), 1);
        assert_eq!(summary(tmp.path()).unwrap().unwrap().endpoint, "abc.myfritz.net:51820");
        assert!(remove(tmp.path()));
        assert!(summary(tmp.path()).is_none() && !remove(tmp.path()));
    }
}
