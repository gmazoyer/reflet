/// Per-peer Graceful Restart state, stored at the speaker level
/// so it persists across session reconnects.
#[derive(Debug, Clone, Default)]
pub struct PeerGrInfo {
    /// Whether the peer advertised GR capability.
    pub supports_gr: bool,
    /// The peer's advertised restart time in seconds.
    pub restart_time: u16,
}

/// Detect if an UPDATE message body is an End-of-RIB marker (RFC 4724).
///
/// Returns `Some((afi, safi))` if the body represents an EoR marker.
///
/// **IPv4 Unicast EoR**: UPDATE with withdrawn_len=0, path_attr_len=0, no NLRI
/// (body is exactly `[0, 0, 0, 0]`).
///
/// **MP EoR** (IPv6, etc.): UPDATE with withdrawn_len=0, a single MP_UNREACH_NLRI
/// attribute (type 15) containing just AFI (2 bytes) + SAFI (1 byte) with no
/// actual withdrawn routes, and no NLRI.
pub fn detect_eor(buf: &[u8], body_len: usize) -> Option<(u16, u8)> {
    if body_len < 4 {
        return None;
    }

    // Parse withdrawn routes length (first 2 bytes)
    let withdrawn_len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if withdrawn_len != 0 {
        return None;
    }

    // Parse total path attributes length (next 2 bytes after withdrawn)
    let attr_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;

    // Check for IPv4 Unicast EoR: no attributes and no NLRI
    if attr_len == 0 {
        // Verify there's no NLRI data after the 4-byte header
        if body_len == 4 {
            return Some((1, 1)); // AFI=1 (IPv4), SAFI=1 (Unicast)
        }
        return None;
    }

    // Check for MP EoR: single MP_UNREACH_NLRI with just AFI+SAFI
    let attr_start = 4;
    let attr_end = attr_start + attr_len;

    // Verify no NLRI after attributes
    if body_len != attr_end {
        return None;
    }

    // Parse the single attribute
    if attr_len < 3 {
        return None;
    }

    let flags = buf[attr_start];
    let attr_type = buf[attr_start + 1];

    // Must be MP_UNREACH_NLRI (type 15)
    if attr_type != 15 {
        return None;
    }

    // Determine attribute value length (extended length bit)
    let (value_offset, value_len) = if flags & 0x10 != 0 {
        // Extended length (2 bytes)
        if attr_len < 4 {
            return None;
        }
        let len = u16::from_be_bytes([buf[attr_start + 2], buf[attr_start + 3]]) as usize;
        (attr_start + 4, len)
    } else {
        // Regular length (1 byte)
        let len = buf[attr_start + 2] as usize;
        (attr_start + 3, len)
    };

    // MP_UNREACH EoR has exactly 3 bytes: AFI (2) + SAFI (1)
    if value_len != 3 {
        return None;
    }

    if value_offset + 3 > buf.len() {
        return None;
    }

    let afi = u16::from_be_bytes([buf[value_offset], buf[value_offset + 1]]);
    let safi = buf[value_offset + 2];

    Some((afi, safi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_eor_ipv4_unicast() {
        // UPDATE with withdrawn_len=0, attr_len=0, no NLRI
        let buf = [0, 0, 0, 0];
        assert_eq!(detect_eor(&buf, 4), Some((1, 1)));
    }

    #[test]
    fn detect_eor_ipv6_unicast() {
        // UPDATE with withdrawn_len=0, single MP_UNREACH_NLRI(AFI=2, SAFI=1)
        // Attribute: flags=0x90 (optional, transitive, extended length? No — 0x80 optional non-transitive)
        // Actually MP_UNREACH_NLRI is optional non-transitive: flags = 0x80 | 0x40 = 0xC0? No.
        // Flags: Optional=1, Transitive=0 → 0x80. Type=15. Length=3. Data: AFI=2, SAFI=1.
        let buf = [
            0, 0, // withdrawn_len = 0
            0, 6,    // attr_len = 6 (flags + type + len + 3 bytes value)
            0x80, // flags: optional, non-transitive
            15,   // type: MP_UNREACH_NLRI
            3,    // length: 3
            0, 2, // AFI = 2 (IPv6)
            1, // SAFI = 1 (Unicast)
        ];
        assert_eq!(detect_eor(&buf, 10), Some((2, 1)));
    }

    #[test]
    fn detect_eor_not_eor() {
        // Normal UPDATE with some attributes (attr_len > 0 but not MP_UNREACH EoR pattern)
        let buf = [
            0, 0, // withdrawn_len = 0
            0, 4,    // attr_len = 4
            0x40, // flags: transitive
            1,    // type: ORIGIN (not MP_UNREACH)
            1,    // length: 1
            0,    // value: IGP
        ];
        assert_eq!(detect_eor(&buf, 8), None);
    }

    #[test]
    fn detect_eor_withdrawal_only() {
        // UPDATE with withdrawn routes only (non-zero withdrawn_len)
        let buf = [
            0, 5, // withdrawn_len = 5
            24, 10, 0,
            0, // withdrawn prefix 10.0.0.0/24 (incomplete but enough for detection)
            0, // extra byte
            0, 0, // attr_len = 0
        ];
        assert_eq!(detect_eor(&buf, 9), None);
    }

    #[test]
    fn detect_eor_ipv4_with_nlri_not_eor() {
        // body_len > 4 with attr_len=0 means there's NLRI → not EoR
        let buf = [
            0, 0, // withdrawn_len = 0
            0, 0, // attr_len = 0
            24, 10, 0, 0, // NLRI: 10.0.0.0/24
        ];
        assert_eq!(detect_eor(&buf, 8), None);
    }

    #[test]
    fn detect_eor_mp_unreach_with_routes_not_eor() {
        // MP_UNREACH_NLRI with more than 3 bytes (has actual withdrawn routes)
        let buf = [
            0, 0, // withdrawn_len = 0
            0, 10,   // attr_len = 10
            0x80, // flags: optional
            15,   // type: MP_UNREACH_NLRI
            7,    // length: 7 (3 AFI/SAFI + 4 bytes withdrawn routes)
            0, 2, // AFI = 2
            1, // SAFI = 1
            32, 0x20, 0x01, 0x0d, // some withdrawn prefix bytes
        ];
        assert_eq!(detect_eor(&buf, 14), None);
    }
}
