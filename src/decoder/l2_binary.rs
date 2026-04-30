//! Binary-format Layer-2 data variants and the SENDAI special case
//! (text-prefixed variants that use a different wire layout).

use super::gps_emit::{emit_gps_lat_lon_raw, emit_gps_speed_dir_status, insert_wgs84};
use crate::gps::decode_degree_bcd;
use crate::json::{FieldEmitter, hex_value};
use crate::primitives::Cursor;
use crate::state::DecoderState;
use serde_json::{Map, Value};

/// SENDAI-flavoured text + hex format used by certain `len` values
/// (63 / 79 / 84 / 121 / 312) when `l2blocks[len-3] == 3`.
pub(super) fn proc_l2block_sendai(
    block: &mut Map<String, Value>,
    state: &DecoderState,
    c: &mut Cursor,
    len: i32,
) {
    let mut e = FieldEmitter::new(block, c);
    e.hex("source_id", 2);
    e.hex("info_type", 1);
    e.hex("destination_id", 2);
    e.hex("hex01", 3);
    e.hex("hex02", 4);
    e.hex("hex03", 4);
    e.hex("hex04", 3);

    match state.l2blocks[39] {
        0x10 | 0x11 | 0x13 | 0x14 | 0x15 | 0x18 | 0x1c | 0x21 | 0x23 | 0x24 | 0x25 | 0x27
        | 0x2a | 0x2b | 0x2e => {
            e.text("date", 11);
            e.text("body_time", 8);
            e.text("body_text01", 6);
            e.rest_hex("hex05");
        }
        0x04 => {
            e.text("body_text01", 4);
            e.rest_hex("hex05");
        }
        0x2c | 0x2d => {
            e.text("date", 11);
            e.text("body_time", 8);
            e.text("body_text01", 1);
            e.rest_hex("hex05");
        }
        0x1b | 0x1d => {
            e.text("date1", 11);
            e.text("time1", 8);
            e.text("body_text01", 6);
            e.text("text03", 9);
            let la = decode_degree_bcd(e.take(11), true, true);
            let lo = decode_degree_bcd(e.take(11), true, true);
            insert_wgs84(e.obj, "gps", la, lo);
            e.text("date2", 11);
            e.text("time2", 8);
            e.text("date3", 11);
            e.text("time3", 8);
            e.text("text05", 6);
            e.text("text06", 5);
            e.text("date4_year", 2);
            e.skip(4);
            e.text("time4", 2);
            e.skip(4);
            e.text("text07", 12);
            e.text("type", 12);
            e.text("subtype", 12);
            e.text("address", 48);
            e.text("name", 24);
            e.text("mapinfo", 24);
            e.text("text08", 4);
            e.rest_hex("hex05");
        }
        0xdd | 0xeb => {
            e.hex("hex05", 31);
            e.hex("hex06", 31);
            e.rest_hex("hex07");
        }
        _ => {
            if len == 63 {
                e.text("date", 11);
                e.text("body_time", 8);
                e.text("body_text01", 6);
                e.rest_hex("hex05");
            } else {
                e.rest_hex("hex05");
            }
        }
    }
}

pub(super) fn proc_l2block_binary(
    block: &mut Map<String, Value>,
    state: &DecoderState,
    c: &mut Cursor,
    len: i32,
) {
    let mut e = FieldEmitter::new(block, c);
    e.hex("source_id", 2);
    e.hex("info_type", 1);

    if len == 44 {
        e.hex("hex01", 2);
        e.hex("hex02", 1);
        e.hex("hex03", 1);
        return;
    }

    match state.l2blocks[39] {
        0x20 => {
            e.text("body_text01", 25);
            e.text("datetime1_year", 2);
            e.skip(8);
            e.text("datetime2_year", 2);
            e.skip(8);
            e.text("name", 30);
            e.text("telephone", 16);
            e.text("text05", 19);
            e.text("text06", 19);
            e.text("text07", 80);
            e.rest_text("text08");
        }
        0x01 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            e.hex("date_year", 1);
            e.skip(2);
            e.hex("body_time", 1);
            e.skip(2);
            e.hex("hex04", 1);
            e.hex("hex05", 1);
            if len > 63 {
                e.hex("hex06", 3);
                emit_gps_lat_lon_raw(&mut e);
                emit_gps_speed_dir_status(&mut e);
                e.hex("hex07", 1);
                e.rest_hex("hex08");
            } else {
                e.hex("hex06", 3);
                e.rest_hex("hex07");
            }
        }
        0x11 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            e.hex("date_year", 1);
            e.skip(2);
            e.hex("body_time", 1);
            e.skip(2);
            e.hex("hex04", 2);
            e.rest_hex("hex05");
        }
        0x0e => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            e.hex("hex04", 3);
            e.hex("hex05", 1);
            e.rest_text("text02");
        }
        0x08 | 0x0a => {
            emit_gps_lat_lon_raw(&mut e);
            emit_gps_speed_dir_status(&mut e);
            e.rest_hex("hex01");
        }
        0x00 | 0x04 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            e.hex("date_year", 1);
            e.skip(2);
            e.hex("body_time", 1);
            e.skip(2);
            e.hex("hex04", 1);
            e.hex("hex05", 1);
            e.hex("hex06", 3);
            if e.peek_u8() != 0 {
                emit_gps_lat_lon_raw(&mut e);
                emit_gps_speed_dir_status(&mut e);
                e.rest_hex("hex07");
            } else {
                e.rest_hex("hex07");
            }
        }
        0x80 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            e.hex("date_year", 1);
            e.skip(2);
            e.hex("body_time", 1);
            e.skip(2);
            emit_gps_lat_lon_raw(&mut e);
            e.hex("hex04", 3);
            e.text("action", 12);
            e.text("cause", 12);
            e.text("address", 48);
            e.text("name", 24);
            e.text("mapinfo", 24);
            let bcd = e.take(2);
            let n = (((bcd[0] >> 4) as u32) * 1000)
                + ((bcd[0] & 0xf) as u32) * 100
                + ((bcd[1] >> 4) as u32) * 10
                + (bcd[1] & 0xf) as u32;
            e.obj.insert("vehicle_count".to_string(), hex_value(bcd));
            for _ in 0..n {
                e.hex("vehicle_id", 2);
            }
        }
        0x10 | 0x15 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
            e.hex("hex03", 1);
            if len == 48 {
                e.hex("hex04", 4);
            } else if matches!(len, 55 | 60) {
                if state.l2blocks[37] == 0xaa && state.l2blocks[38] == 0xaa {
                    e.rest_hex("hex04");
                } else {
                    emit_gps_lat_lon_raw(&mut e);
                    emit_gps_speed_dir_status(&mut e);
                    e.rest_hex("hex04");
                }
            } else {
                e.hex("hex04", 2);
                let len_byte = e.take(1);
                let n = len_byte[0] as usize;
                e.obj
                    .insert("data_len_hex".to_string(), hex_value(len_byte));
                e.hex("data_hex", n);
                e.hex("hex05", 1);
            }
        }
        0x83 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 1);
        }
        0x25 | 0x3c | 0xf0 => {
            e.rest_hex("hex01");
        }
        0x12 | 0x13 | 0x14 => {
            e.hex("destination_id", 2);
            e.hex("hex02", 2);
            e.hex("hex03", 2);
            let len_byte = e.take(1);
            let n = len_byte[0] as usize;
            e.obj
                .insert("data_len_hex".to_string(), hex_value(len_byte));
            e.hex("data_hex", n);
            e.hex("hex04", 1);
        }
        _ => {
            e.rest_hex("hex01");
        }
    }
}
