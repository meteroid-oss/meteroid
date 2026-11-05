use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True when an address must never be used as an egress destination:
/// loopback, link-local, RFC1918/ULA, CGNAT, benchmarking, multicast,
/// reserved space and the cloud metadata ranges reachable through them.
pub(crate) fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        IpAddr::V6(v6) => is_disallowed_ipv6(v6),
    }
}

fn is_disallowed_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    match o[0] {
        0 | 10 | 127 => true,
        100 => o[1] & 0xc0 == 64,
        169 => o[1] == 254,
        172 => o[1] & 0xf0 == 16,
        192 => (o[1] == 0 && o[2] == 0) || o[1] == 168,
        198 => o[1] & 0xfe == 18,
        224..=255 => true,
        _ => false,
    }
}

fn is_disallowed_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() {
        return true;
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_disallowed_ipv4(mapped);
    }
    let segments = ip.segments();
    if let Some(embedded) = embedded_ipv4(&segments) {
        return is_disallowed_ipv4(embedded);
    }
    let first_byte = ip.octets()[0];
    first_byte & 0xfe == 0xfc
        || segments[0] & 0xffc0 == 0xfe80
        || first_byte == 0xff
        || is_opaque_tunnel(&segments)
}

/// Local-use NAT64 (64:ff9b:1::/48) and Teredo (2001::/32) map to IPv4 destinations
/// that cannot be read reliably from the address, so both are refused outright.
fn is_opaque_tunnel(segments: &[u16; 8]) -> bool {
    matches!(
        segments,
        [0x0064, 0xff9b, 0x0001, ..] | [0x2001, 0x0000, ..]
    )
}

/// IPv4 addresses carried in transition prefixes: NAT64 (64:ff9b::/96), the deprecated
/// IPv4-compatible form (::/96) and 6to4 (2002::/16).
fn embedded_ipv4(segments: &[u16; 8]) -> Option<Ipv4Addr> {
    let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    match *segments {
        [0x0064, 0xff9b, 0, 0, 0, 0, hi, lo] => Some(v4(hi, lo)),
        [0, 0, 0, 0, 0, 0, hi, lo] => Some(v4(hi, lo)),
        [0x2002, hi, lo, ..] => Some(v4(hi, lo)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn disallowed(s: &str) -> bool {
        is_disallowed_ip(IpAddr::from_str(s).unwrap())
    }

    #[test]
    fn ipv4_reserved_ranges_are_rejected() {
        for addr in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(disallowed(addr), "{addr} should be disallowed");
        }
    }

    #[test]
    fn ipv4_public_ranges_are_allowed() {
        for addr in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.0.1.1",
            "192.167.255.255",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
        ] {
            assert!(!disallowed(addr), "{addr} should be allowed");
        }
    }

    #[test]
    fn ipv6_reserved_ranges_are_rejected() {
        for addr in [
            "::",
            "::1",
            "fc00::1",
            "fd00::1",
            "fd00:ec2::254",
            "fe80::1",
            "febf::1",
            "ff02::1",
        ] {
            assert!(disallowed(addr), "{addr} should be disallowed");
        }
    }

    #[test]
    fn ipv6_public_ranges_are_allowed() {
        for addr in ["2606:4700:4700::1111", "2001:4860:4860::8888", "fec0::1"] {
            assert!(!disallowed(addr), "{addr} should be allowed");
        }
    }

    #[test]
    fn ipv4_mapped_v6_follows_the_v4_rules() {
        assert!(disallowed("::ffff:127.0.0.1"));
        assert!(disallowed("::ffff:169.254.169.254"));
        assert!(disallowed("::ffff:10.0.0.1"));
        assert!(!disallowed("::ffff:8.8.8.8"));
    }

    #[test]
    fn nat64_embedded_v4_follows_the_v4_rules() {
        assert!(disallowed("64:ff9b::127.0.0.1"));
        assert!(disallowed("64:ff9b::169.254.169.254"));
        assert!(!disallowed("64:ff9b::8.8.8.8"));
        // Same embedded v4 but a different prefix: not a NAT64 translation.
        assert!(!disallowed("64:ff9a::7f00:1"));
    }

    #[test]
    fn local_use_nat64_and_teredo_are_rejected() {
        for addr in [
            "64:ff9b:1::1",
            "64:ff9b:1:ffff::808:808",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(disallowed(addr), "{addr} should be disallowed");
        }
        for addr in ["64:ff9b:2::1", "2001:1::1", "2001:db9::1"] {
            assert!(!disallowed(addr), "{addr} should be allowed");
        }
    }

    #[test]
    fn ipv4_compatible_and_6to4_embeddings_follow_the_v4_rules() {
        assert!(disallowed("::127.0.0.1"));
        assert!(disallowed("::10.0.0.1"));
        assert!(!disallowed("::8.8.8.8"));
        assert!(disallowed("2002:7f00:1::"));
        assert!(disallowed("2002:a9fe:a9fe::1"));
        assert!(!disallowed("2002:808:808::1"));
    }
}
