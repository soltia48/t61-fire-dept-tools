//! TKY (Tokyo) datum coordinate decoding, TKY → WGS84 conversion,
//! and ACCH command/type/subcommand name lookups.
//!
//! All coordinates are represented as degrees * 1e6, with `0` as the
//! invalid sentinel.

/// 24-bit raw latitude (BCD-like) -> TKY degrees * 1e6.
pub fn decode_latitude_24(l: u32) -> u32 {
    if l == 0 {
        return 0;
    }
    let l1 = l / 360_000;
    let l2 = l % 360_000;
    (l1 + 10) * 1_000_000 + ((l2 * 50 + 9) / 18)
}

/// 24-bit raw longitude (BCD-like) -> TKY degrees * 1e6.
pub fn decode_longitude_24(l: u32) -> u32 {
    if l == 0 {
        return 0;
    }
    let l1 = l / 360_000;
    let l2 = l % 360_000;
    (l1 + 110) * 1_000_000 + ((l2 * 50 + 9) / 18)
}

/// 9-byte ASCII-BCD "DDMMSSSSS" degree string -> TKY degrees * 1e6.
///
/// `skip_after_3` / `skip_after_5` model the original `print_degree2`
/// variant which drops separator bytes after positions 3 and 5.
pub fn decode_degree_bcd(p: &[u8], skip_after_3: bool, skip_after_5: bool) -> u32 {
    let mut idx = 0usize;
    let digit = |i| (p[i] - b'0') as u32;

    let deg = digit(idx) * 100 + digit(idx + 1) * 10 + digit(idx + 2);
    idx += 3;
    if skip_after_3 {
        idx += 1;
    }
    if deg == 0 {
        return 0;
    }
    let mut frac = digit(idx) * 60_000 + digit(idx + 1) * 6_000;
    idx += 2;
    if skip_after_5 {
        idx += 1;
    }
    frac += digit(idx) * 1_000 + digit(idx + 1) * 100 + digit(idx + 2) * 10 + digit(idx + 3);
    frac = (frac * 50 + 9) / 2 / 9;
    deg * 1_000_000 + frac
}

pub fn tky_to_wgs84_lat(la: u32, lo: u32) -> u32 {
    if la == 0 || lo == 0 {
        return 0;
    }
    la - (la / 9_350) + (lo / 57_261) + 4_602
}

pub fn tky_to_wgs84_lon(la: u32, lo: u32) -> u32 {
    if la == 0 || lo == 0 {
        return 0;
    }
    lo - (la / 21_721) - (lo / 12_042) + 10_040
}

pub fn gps_status_name(stat: u8) -> Option<&'static str> {
    match stat & 0xf {
        0 => Some("NG"),
        1 => Some("OK"),
        3 => Some("GOOD"),
        _ => None,
    }
}

pub fn acch_signal_subcommand_name(subcommand: u8) -> Option<&'static str> {
    match subcommand {
        0 => Some("off"),
        1 => Some("on"),
        _ => None,
    }
}

pub fn acch_signal_type_name(subcommand: u8, ty: u8) -> Option<&'static str> {
    match ty & 0xf {
        0 => match subcommand {
            0 => Some("stop"),
            1 => Some("notify"),
            _ => None,
        },
        1 => Some("fire"),
        2 => Some("ambulance"),
        3 => Some("rescue"),
        4 => Some("otherwise"),
        5 => Some("PA cooperation"),
        7 => Some("disaster response"),
        _ => None,
    }
}
