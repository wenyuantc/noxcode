//! Pure address and explicit NO_PROXY policy; no DNS or implicit environment reads.
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Conservative public-egress classification. Special-use transition networks
/// require a grant even where a particular deployment makes them reachable.
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_ipv4(address),
        IpAddr::V6(address) => public_ipv6(address),
    }
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, c, d] = address.octets();
    match (a, b, c) {
        (0 | 10 | 127, _, _)
        | (100, 64..=127, _)
        | (169, 254, _)
        | (172, 16..=31, _)
        | (192, 168, _)
        | (192, 0, 2)
        | (192, 88, 99)
        | (198, 18..=19, _)
        | (198, 51, 100)
        | (203, 0, 113) => false,
        // PCP and TURN anycast are the globally reachable exceptions in this
        // otherwise special-purpose /24 (RFC 7723 and RFC 8155).
        (192, 0, 0) => matches!(d, 9 | 10),
        _ => a < 224,
    }
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    let bytes = address.octets();
    let mapped = bytes[..10] == [0; 10] && bytes[10..12] == [0xff, 0xff];
    let well_known_nat64 = bytes[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0];
    if mapped || well_known_nat64 {
        return public_ipv4(Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]));
    }
    let segments = address.segments();
    // Only global unicast is automatic. Exclude IETF protocol assignments
    // (2001::/23), documentation, and 6to4 transition addresses, whose
    // deployment/relay route is not a direct public-egress guarantee.
    segments[0] & 0xe000 == 0x2000
        && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] < 0x1000)
}

/// Match reqwest's documented NO_PROXY grammar against the URL hostname,
/// never against a separately resolved address. Route selection must use the
/// same result as the actual per-request client.
pub fn bypass_proxy(host: &str, rules: Option<&str>) -> bool {
    let host = host
        .trim_matches(&['[', ']'][..])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let address = host.parse::<IpAddr>().ok();
    rules.unwrap_or_default().split(',').any(|rule| {
        let rule = rule.trim();
        if rule.is_empty() {
            return false;
        }
        if rule == "*" {
            return true;
        }
        if let Some((network, prefix)) = rule.split_once('/') {
            return match (
                address,
                network.trim_matches(&['[', ']'][..]).parse::<IpAddr>(),
                prefix.parse::<u32>(),
            ) {
                (Some(IpAddr::V4(ip)), Ok(IpAddr::V4(network)), Ok(prefix)) if prefix <= 32 => {
                    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
                    u32::from(ip) & mask == u32::from(network) & mask
                }
                (Some(IpAddr::V6(ip)), Ok(IpAddr::V6(network)), Ok(prefix)) if prefix <= 128 => {
                    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
                    u128::from(ip) & mask == u128::from(network) & mask
                }
                _ => false,
            };
        }
        if let Ok(network) = rule.trim_matches(&['[', ']'][..]).parse::<IpAddr>() {
            return address == Some(network);
        }
        let domain = rule
            .trim_start_matches('.')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        !domain.is_empty()
            && (host == domain
                || host
                    .strip_suffix(&domain)
                    .is_some_and(|prefix| prefix.ends_with('.')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_use_ipv4_is_not_public_egress() {
        for address in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.170",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn public_addresses_and_range_boundaries_remain_available() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.0.9",
            "192.0.0.10",
            "198.17.255.255",
            "198.20.0.0",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public_address(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn ipv6_private_and_embedded_private_addresses_require_grants() {
        for address in [
            "::",
            "::1",
            "::8.8.8.8",
            "fc00::1",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:2::1",
            "2001:10::1",
            "2001:20::1",
            "100::1",
            "64:ff9b:1::1",
            "3fff::1",
            "3fff:fff:ffff::1",
            "2002:7f00:1::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(is_public_address(address.parse().unwrap()), "{address}");
        }
    }

    #[test]
    fn no_proxy_domain_rules_respect_label_boundaries() {
        let rules = Some(" .example.com, localhost ");
        assert!(bypass_proxy("example.com", rules));
        assert!(bypass_proxy("API.EXAMPLE.COM.", rules));
        assert!(bypass_proxy("localhost", rules));
        assert!(!bypass_proxy("notexample.com", rules));
        assert!(!bypass_proxy("example.com.attacker.test", rules));
        assert!(!bypass_proxy("api.example.com", Some("*.example.com")));
        assert!(!bypass_proxy("example.com", None));
        // Documented host grammar does not invent port-specific entries.
        assert!(!bypass_proxy("example.com", Some("example.com:443")));
        assert!(!bypass_proxy("192.168.0.7", Some("192.168.0.7:80")));
        assert!(bypass_proxy("anywhere.test", Some("*")));
    }

    #[test]
    fn no_proxy_ip_and_cidr_rules_cover_both_families() {
        let rules = Some("10.0.0.0/8, 192.168.0.7, ::1, fd00::/8");
        for host in ["10.99.1.1", "192.168.0.7", "[::1]", "fd12::abcd"] {
            assert!(bypass_proxy(host, rules), "{host}");
        }
        for host in ["11.0.0.1", "192.168.0.8", "2001:4860::1", "internal.test"] {
            assert!(!bypass_proxy(host, rules), "{host}");
        }
        assert!(bypass_proxy("8.8.8.8", Some("0.0.0.0/0")));
        assert!(bypass_proxy("::1", Some("::/0")));
        assert!(!bypass_proxy("8.8.8.8", Some("0.0.0.0/33")));
        assert!(!bypass_proxy("::1", Some("::/129")));
    }
}
