//! Working out a client's real IP address when the server sits behind a
//! reverse proxy.
//!
//! Every connection's TCP peer is the proxy, not the player, so the address
//! worth logging (or rate-limiting on) only exists in a forwarding header.
//! Those headers are client-supplied text: anyone who can reach the port can
//! claim any address they like.  So they are ignored entirely unless the
//! deployment says which peers are allowed to set them, through the
//! environment:
//!
//! * `REMOTE_WEBGPU_TRUSTED_PROXIES` — comma-separated IPs and CIDR blocks
//!   whose forwarding headers are believed (`10.0.0.0/8, 192.168.1.7`), or
//!   the word `any` to believe every peer.  Unset or empty (the default)
//!   turns the whole mechanism off: the TCP peer is the client, full stop.
//! * `REMOTE_WEBGPU_FORWARDED_HEADER` — which header carries the chain;
//!   `x-forwarded-for` by default.  `x-real-ip` (or any other header holding
//!   a comma-separated list of addresses, closest-to-the-client first) works
//!   too.  RFC 7239 `Forwarded:` syntax is *not* parsed.
//! * `REMOTE_WEBGPU_TRUSTED_PROXY_HOPS` — how many proxies of your own sit
//!   in front of the server, i.e. how many entries to skip from the right of
//!   the chain; 1 by default, which is the single-nginx case.
//!
//! `any` is only ever right when nothing but the proxy can reach the port
//! (a container network, a loopback bind); on a publicly reachable port it
//! lets any client forge its address.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// One entry of `REMOTE_WEBGPU_TRUSTED_PROXIES`: an address with a prefix
/// length (a bare address is just a full-length prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrustedNet {
    addr: IpAddr,
    prefix: u32,
}

impl TrustedNet {
    fn parse(text: &str) -> Result<TrustedNet, String> {
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (
                addr,
                Some(
                    prefix
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| format!("{text:?} has a bad prefix length"))?,
                ),
            ),
            None => (text, None),
        };
        let addr: IpAddr = addr
            .trim()
            .parse()
            .map_err(|_| format!("{text:?} is not an IP address or CIDR block"))?;
        let bits = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(bits);
        if prefix > bits {
            return Err(format!("{text:?} has a prefix longer than the address"));
        }
        Ok(TrustedNet { addr, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        // An IPv4 proxy reaching a dual-stack listener shows up as
        // ::ffff:a.b.c.d, so compare in whichever family the entry is
        // written in.
        let ip = match (self.addr, ip) {
            (IpAddr::V4(_), IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => return false,
            },
            (IpAddr::V6(_), IpAddr::V4(v4)) => IpAddr::V6(v4.to_ipv6_mapped()),
            _ => ip,
        };
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                prefix_eq(&net.octets(), &ip.octets(), self.prefix)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                prefix_eq(&net.octets(), &ip.octets(), self.prefix)
            }
            _ => false,
        }
    }
}

/// Do two addresses agree on their first `prefix` bits?
fn prefix_eq(a: &[u8], b: &[u8], prefix: u32) -> bool {
    let whole = (prefix / 8) as usize;
    if a[..whole] != b[..whole] {
        return false;
    }
    let rest = prefix % 8;
    if rest == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - rest);
    a[whole] & mask == b[whole] & mask
}

/// Which peers may speak for their clients, and how.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Lowercased name of the header carrying the forwarding chain.
    header: String,
    /// Peers whose header is believed.  Empty (and `trust_any` false) means
    /// no header is ever believed.
    trusted: Vec<TrustedNet>,
    trust_any: bool,
    /// Proxies of our own at the right-hand end of the chain.
    hops: usize,
}

impl ProxyConfig {
    /// Trust nothing: the TCP peer is always the client.
    pub fn disabled() -> ProxyConfig {
        ProxyConfig {
            header: "x-forwarded-for".to_string(),
            trusted: Vec::new(),
            trust_any: false,
            hops: 1,
        }
    }

    /// Read the configuration described in the module docs.  A malformed
    /// value is a startup panic rather than a silently ignored setting: an
    /// address policy that quietly does nothing is worse than not booting.
    pub fn from_env() -> ProxyConfig {
        Self::parse(
            std::env::var("REMOTE_WEBGPU_TRUSTED_PROXIES").ok().as_deref(),
            std::env::var("REMOTE_WEBGPU_FORWARDED_HEADER").ok().as_deref(),
            std::env::var("REMOTE_WEBGPU_TRUSTED_PROXY_HOPS").ok().as_deref(),
        )
        .unwrap_or_else(|error| panic!("remote-wgpu: {error}"))
    }

    fn parse(
        proxies: Option<&str>,
        header: Option<&str>,
        hops: Option<&str>,
    ) -> Result<ProxyConfig, String> {
        let mut config = ProxyConfig::disabled();
        if let Some(header) = header {
            let header = header.trim().trim_end_matches(':').to_ascii_lowercase();
            if header.is_empty() {
                return Err("REMOTE_WEBGPU_FORWARDED_HEADER is empty".to_string());
            }
            config.header = header;
        }
        if let Some(hops) = hops {
            config.hops = hops
                .trim()
                .parse()
                .ok()
                .filter(|hops| *hops >= 1)
                .ok_or_else(|| {
                    format!("REMOTE_WEBGPU_TRUSTED_PROXY_HOPS={hops:?} is not a count of 1 or more")
                })?;
        }
        for entry in proxies.unwrap_or("").split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            if entry.eq_ignore_ascii_case("any") {
                config.trust_any = true;
                continue;
            }
            config.trusted.push(
                TrustedNet::parse(entry)
                    .map_err(|error| format!("REMOTE_WEBGPU_TRUSTED_PROXIES: {error}"))?,
            );
        }
        Ok(config)
    }

    /// True when at least one peer's forwarding header is believed.
    pub fn is_enabled(&self) -> bool {
        self.trust_any || !self.trusted.is_empty()
    }

    /// One line describing the policy, for the startup banner.
    pub fn describe(&self) -> String {
        if !self.is_enabled() {
            return "reverse-proxy headers ignored (set REMOTE_WEBGPU_TRUSTED_PROXIES to honour \
                    them)"
                .to_string();
        }
        let from = if self.trust_any {
            "any peer".to_string()
        } else {
            let list: Vec<String> = self
                .trusted
                .iter()
                .map(|net| format!("{}/{}", net.addr, net.prefix))
                .collect();
            list.join(", ")
        };
        format!(
            "trusting the {} header from {} ({} proxy hop(s))",
            self.header, from, self.hops
        )
    }

    fn trusts(&self, peer: IpAddr) -> bool {
        self.trust_any || self.trusted.iter().any(|net| net.contains(peer))
    }

    /// The client address `head` claims, if `peer` is allowed to claim one.
    ///
    /// `head` is the raw HTTP request head (the websocket upgrade, or a
    /// plain `GET`).  Returns `None` whenever the peer is untrusted, the
    /// header is absent, or the chain is too short for the configured
    /// number of hops — in every one of those cases the caller keeps the
    /// TCP peer as the client address.
    pub fn client_ip(&self, peer: IpAddr, head: &str) -> Option<IpAddr> {
        if !self.trusts(peer) {
            return None;
        }
        let chain = header_values(head, &self.header);
        // The rightmost `hops` entries were written by our own proxies; the
        // one just left of them is what the closest proxy saw.  A chain too
        // short for that means the request did not come through the proxy
        // chain the configuration describes, so believe none of it.
        let index = chain.len().checked_sub(self.hops)?;
        parse_ip(chain[index])
    }
}

/// Every comma-separated value of `name` in an HTTP request head, in order,
/// across repeated occurrences of the header.
fn header_values<'a>(head: &'a str, name: &str) -> Vec<&'a str> {
    let mut values = Vec::new();
    // Skip the request line; stop at the blank line ending the head (the
    // peeked buffer may hold the start of a body, or nothing at all).
    for line in head.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case(name) {
            continue;
        }
        values.extend(value.split(',').map(str::trim).filter(|v| !v.is_empty()));
    }
    values
}

/// One chain entry to an address: bare IPs, `host:port` and `[v6]:port` all
/// appear in the wild, and `unknown`/obfuscated identifiers are not
/// addresses at all.
fn parse_ip(text: &str) -> Option<IpAddr> {
    let text = text.trim();
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Some(addr.ip());
    }
    // "[2001:db8::1]" without a port.
    if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return inner.parse().ok();
    }
    // "1.2.3.4:8080" is a SocketAddr above; a bare IPv4 with a trailing
    // port and nothing else is all that is left worth trying.
    let (host, _) = text.rsplit_once(':')?;
    host.parse::<Ipv4Addr>().ok().map(IpAddr::V4)
}

/// Where one connection came from: the TCP peer, plus the client address a
/// trusted proxy reported for it.
#[derive(Debug, Clone, Copy)]
pub struct ClientAddr {
    peer: Option<SocketAddr>,
    forwarded: Option<IpAddr>,
}

impl ClientAddr {
    pub(crate) fn new(peer: SocketAddr, forwarded: Option<IpAddr>) -> ClientAddr {
        ClientAddr {
            peer: Some(peer),
            forwarded,
        }
    }

    /// The address of a client with no connection at all (the loopback
    /// client behind the no-op backend).
    pub(crate) fn none() -> ClientAddr {
        ClientAddr {
            peer: None,
            forwarded: None,
        }
    }

    /// The socket this connection actually came from: the reverse proxy,
    /// when there is one.
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// The address a *trusted* proxy reported for the client, if any.
    pub fn forwarded_ip(&self) -> Option<IpAddr> {
        self.forwarded
    }

    /// The client's real IP as best the configuration allows it to be
    /// known: what a trusted proxy reported, else the TCP peer's address.
    pub fn ip(&self) -> Option<IpAddr> {
        self.forwarded.or_else(|| self.peer.map(|peer| peer.ip()))
    }
}

impl std::fmt::Display for ClientAddr {
    /// The form used in logs: the client's address, and the proxy it came
    /// through when that is not the same thing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.forwarded, self.peer) {
            (Some(client), Some(peer)) => write!(f, "{client} (via proxy {peer})"),
            (Some(client), None) => write!(f, "{client}"),
            (None, Some(peer)) => write!(f, "{peer}"),
            (None, None) => f.write_str("(no connection)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROXY: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
    const OUTSIDER: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));

    fn head(headers: &str) -> String {
        format!("GET / HTTP/1.1\r\nHost: example.test\r\n{headers}\r\n\r\n")
    }

    fn config(proxies: &str) -> ProxyConfig {
        ProxyConfig::parse(Some(proxies), None, None).unwrap()
    }

    #[test]
    fn headers_are_ignored_without_configuration() {
        let config = ProxyConfig::disabled();
        assert!(!config.is_enabled());
        assert_eq!(
            config.client_ip(PROXY, &head("X-Forwarded-For: 198.51.100.7")),
            None
        );
    }

    #[test]
    fn an_untrusted_peer_cannot_claim_an_address() {
        let config = config("10.0.0.0/8");
        assert_eq!(
            config.client_ip(OUTSIDER, &head("X-Forwarded-For: 198.51.100.7")),
            None
        );
    }

    #[test]
    fn a_trusted_proxy_names_the_client() {
        let config = config("10.0.0.0/8, 192.168.1.7");
        let ip = config.client_ip(PROXY, &head("X-Forwarded-For: 198.51.100.7"));
        assert_eq!(ip, Some("198.51.100.7".parse().unwrap()));
        let ip = config.client_ip(
            "192.168.1.7".parse().unwrap(),
            &head("x-forwarded-for: 2001:db8::5"),
        );
        assert_eq!(ip, Some("2001:db8::5".parse().unwrap()));
    }

    #[test]
    fn only_the_hop_the_proxy_wrote_is_believed() {
        let config = config("10.0.0.0/8");
        // A client that forges a chain entry: our one proxy appended the
        // address it saw, and that is the entry we read.
        let ip = config.client_ip(
            PROXY,
            &head("X-Forwarded-For: 1.2.3.4, 198.51.100.7"),
        );
        assert_eq!(ip, Some("198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn extra_hops_skip_further_left() {
        let config = ProxyConfig::parse(Some("10.0.0.0/8"), None, Some("2")).unwrap();
        let ip = config.client_ip(
            PROXY,
            &head("X-Forwarded-For: 198.51.100.7, 10.0.0.9"),
        );
        assert_eq!(ip, Some("198.51.100.7".parse().unwrap()));
        // Too short for the configured chain: fall back to the peer.
        assert_eq!(config.client_ip(PROXY, &head("X-Forwarded-For: 1.2.3.4")), None);
    }

    #[test]
    fn values_split_across_repeated_headers() {
        let config = config("10.0.0.0/8");
        let ip = config.client_ip(
            PROXY,
            &head("X-Forwarded-For: 1.2.3.4\r\nX-Forwarded-For: 198.51.100.7"),
        );
        assert_eq!(ip, Some("198.51.100.7".parse().unwrap()));
    }

    #[test]
    fn a_custom_header_can_be_named() {
        let config = ProxyConfig::parse(Some("any"), Some("X-Real-IP"), None).unwrap();
        assert_eq!(
            config.client_ip(OUTSIDER, &head("X-Real-IP: 198.51.100.7")),
            Some("198.51.100.7".parse().unwrap())
        );
        // ... and the default header is then not consulted.
        assert_eq!(
            config.client_ip(OUTSIDER, &head("X-Forwarded-For: 198.51.100.7")),
            None
        );
    }

    #[test]
    fn missing_or_unusable_values_fall_back_to_the_peer() {
        let config = config("10.0.0.0/8");
        assert_eq!(config.client_ip(PROXY, &head("Accept: */*")), None);
        assert_eq!(config.client_ip(PROXY, &head("X-Forwarded-For: unknown")), None);
        assert_eq!(config.client_ip(PROXY, &head("X-Forwarded-For:  ")), None);
    }

    #[test]
    fn ports_and_brackets_are_stripped() {
        assert_eq!(parse_ip("198.51.100.7:443"), Some("198.51.100.7".parse().unwrap()));
        assert_eq!(parse_ip("[2001:db8::5]:443"), Some("2001:db8::5".parse().unwrap()));
        assert_eq!(parse_ip("[2001:db8::5]"), Some("2001:db8::5".parse().unwrap()));
        assert_eq!(parse_ip("2001:db8::5"), Some("2001:db8::5".parse().unwrap()));
        assert_eq!(parse_ip("_hidden"), None);
    }

    #[test]
    fn a_dual_stack_peer_matches_an_ipv4_block() {
        let config = config("10.0.0.0/8");
        let mapped: IpAddr = "::ffff:10.0.0.2".parse().unwrap();
        assert_eq!(
            config.client_ip(mapped, &head("X-Forwarded-For: 198.51.100.7")),
            Some("198.51.100.7".parse().unwrap())
        );
    }

    #[test]
    fn prefixes_bound_what_a_block_covers() {
        let config = config("10.1.2.0/24, 2001:db8::/32");
        assert!(config.trusts("10.1.2.255".parse().unwrap()));
        assert!(!config.trusts("10.1.3.0".parse().unwrap()));
        assert!(config.trusts("2001:db8:1234::9".parse().unwrap()));
        assert!(!config.trusts("2001:db9::1".parse().unwrap()));
    }

    #[test]
    fn bad_configuration_is_rejected() {
        assert!(ProxyConfig::parse(Some("10.0.0.0/33"), None, None).is_err());
        assert!(ProxyConfig::parse(Some("not-an-ip"), None, None).is_err());
        assert!(ProxyConfig::parse(None, Some(""), None).is_err());
        assert!(ProxyConfig::parse(None, None, Some("0")).is_err());
        assert!(ProxyConfig::parse(None, None, Some("lots")).is_err());
    }
}
