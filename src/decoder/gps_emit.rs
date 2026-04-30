//! GPS sub-object emitters used by the L2 data-block parsers.
//!
//! Each `emit_gps_*` advances the cursor and inserts a `gps` (or
//! `nav`) sub-object via the [`FieldEmitter`].  [`insert_wgs84`] is
//! exposed as a primitive for callers that already have decoded
//! lat/lon at hand (the SENDAI BCD-with-separators path).

use crate::gps::{
    decode_latitude_24, decode_longitude_24, gps_status_name, tky_to_wgs84_lat, tky_to_wgs84_lon,
};
use crate::json::FieldEmitter;
use crate::tables::DIRECTION_STR;
use serde_json::{Map, Value, json};

pub(super) fn insert_wgs84(obj: &mut Map<String, Value>, key: &str, wa: u32, wo: u32) {
    if wa == 0 || wo == 0 {
        return;
    }
    obj.insert(
        key.to_string(),
        json!({
            "lat": wa as f64 / 1e6,
            "lon": wo as f64 / 1e6,
        }),
    );
}

pub(super) fn emit_gps_lat_lon_bcd(e: &mut FieldEmitter) {
    use crate::gps::decode_degree_bcd;
    let la = decode_degree_bcd(e.c.take(9), false, false);
    let lo = decode_degree_bcd(e.c.take(9), false, false);
    insert_wgs84(
        e.obj,
        "gps",
        tky_to_wgs84_lat(la, lo),
        tky_to_wgs84_lon(la, lo),
    );
}

pub(super) fn emit_gps_lat_lon_raw(e: &mut FieldEmitter) {
    let raw = e.c.take(6);
    let raw_la = ((raw[0] as u32) << 16) | ((raw[1] as u32) << 8) | raw[2] as u32;
    let raw_lo = ((raw[3] as u32) << 16) | ((raw[4] as u32) << 8) | raw[5] as u32;
    let la = decode_latitude_24(raw_la);
    let lo = decode_longitude_24(raw_lo);
    insert_wgs84(
        e.obj,
        "gps",
        tky_to_wgs84_lat(la, lo),
        tky_to_wgs84_lon(la, lo),
    );
}

pub(super) fn emit_gps_speed_dir_status(e: &mut FieldEmitter) {
    let speed = e.c.take_u8();
    let dirstat = e.c.take_u8();
    let dir_idx = (dirstat >> 4) as usize;

    let mut nav = Map::new();
    nav.insert("speed_kmh".to_string(), json!(speed));
    nav.insert("dir_idx".to_string(), json!(dir_idx));
    nav.insert("dir".to_string(), json!(DIRECTION_STR[dir_idx]));
    nav.insert("gps_status_code".to_string(), json!(dirstat & 0xf));
    if let Some(n) = gps_status_name(dirstat) {
        nav.insert("gps_status".to_string(), json!(n));
    }
    e.obj.insert("nav".to_string(), Value::Object(nav));
}
