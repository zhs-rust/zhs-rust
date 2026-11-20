use std::net::{IpAddr, SocketAddr};

use super::context::Context;
use super::pipeline::{Middleware, Next};
use super::service::ServiceFuture;

pub const FORWARDED_FOR: &str = "x-forwarded-for";

#[derive(Clone, Debug)]
pub struct TrustedProxies {
    proxies: Vec<IpAddr>,
    header: String,
}

impl TrustedProxies {
    pub fn new(proxies: impl IntoIterator<Item = IpAddr>) -> Self {
        Self {
            proxies: proxies.into_iter().collect(),
            header: FORWARDED_FOR.to_owned(),
        }
    }

    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header = header.into();
        self
    }

    pub fn is_trusted(&self, address: IpAddr) -> bool {
        self.proxies.contains(&address)
    }

    fn client(&self, value: &str) -> Option<IpAddr> {
        let addresses: Vec<IpAddr> = value.split(',').filter_map(parse_address).collect();

        addresses
            .iter()
            .rev()
            .find(|address| !self.is_trusted(**address))
            .copied()
            .or_else(|| addresses.first().copied())
    }
}

impl Middleware for TrustedProxies {
    fn call(&self, mut context: Context, next: Next) -> ServiceFuture {
        let trusted = context
            .peer_addr()
            .is_some_and(|peer| self.is_trusted(peer.ip()));

        if trusted
            && let Some(client) = context
                .headers()
                .get(&self.header)
                .and_then(|value| self.client(value))
        {
            context
                .request_mut()
                .set_peer_addr(SocketAddr::new(client, 0));
        }

        next.run(context)
    }
}

fn parse_address(entry: &str) -> Option<IpAddr> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }

    if let Ok(address) = entry.parse::<IpAddr>() {
        return Some(address);
    }
    if let Ok(socket) = entry.parse::<SocketAddr>() {
        return Some(socket.ip());
    }

    let rest = entry.strip_prefix('[')?;
    let (address, _) = rest.split_once(']')?;
    address.parse::<IpAddr>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxies() -> TrustedProxies {
        TrustedProxies::new([IpAddr::from([127, 0, 0, 1]), IpAddr::from([10, 0, 0, 1])])
    }

    #[test]
    fn takes_the_rightmost_untrusted_address() {
        let proxies = proxies();

        assert_eq!(
            proxies.client("203.0.113.7, 10.0.0.1, 192.0.2.5"),
            Some(IpAddr::from([192, 0, 2, 5]))
        );
        assert_eq!(
            proxies.client("203.0.113.7"),
            Some(IpAddr::from([203, 0, 113, 7]))
        );
    }

    #[test]
    fn skips_trusted_hops_from_the_right() {
        let proxies = proxies();
        assert_eq!(
            proxies.client("198.51.100.4, 192.0.2.9, 10.0.0.1, 127.0.0.1"),
            Some(IpAddr::from([192, 0, 2, 9]))
        );
    }

    #[test]
    fn falls_back_to_the_leftmost_when_every_hop_is_trusted() {
        let proxies = proxies();
        assert_eq!(
            proxies.client("10.0.0.1, 127.0.0.1"),
            Some(IpAddr::from([10, 0, 0, 1]))
        );
    }

    #[test]
    fn parses_ports_and_ipv6() {
        assert_eq!(
            parse_address("192.0.2.5:443"),
            Some(IpAddr::from([192, 0, 2, 5]))
        );
        assert_eq!(
            parse_address("[2001:db8::1]:443"),
            Some("2001:db8::1".parse().unwrap())
        );
        assert_eq!(
            parse_address("2001:db8::1"),
            Some("2001:db8::1".parse().unwrap())
        );
        assert_eq!(parse_address("not-an-ip"), None);
    }
}
