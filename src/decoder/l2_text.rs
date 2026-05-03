//! Text-format Layer-2 data variants (info_type_byte in 0x30..=0x39).
//!
//! The info-type byte at `l2blocks[40..42]` is interpreted as two
//! ASCII hex digits, giving an `infotype` in 0x00..0xff that selects
//! one of the per-format layouts below.

use super::gps_emit::emit_gps_lat_lon_bcd;
use crate::json::{FieldEmitter, text_value};
use crate::primitives::{Cursor, parse_2digit, parse_3digit};
use crate::state::DecoderState;
use serde_json::{Map, Value};

pub(super) fn proc_l2block_data_textinfo(
    block: &mut Map<String, Value>,
    state: &DecoderState,
    c: &mut Cursor,
    len: i32,
) {
    // info-type byte encoded as two ASCII hex digits at l2blocks[40..42];
    // letters A-F (>64) subtract 55, digits 0-9 subtract 48.
    let infotype = {
        let h = state.l2blocks[40];
        let l = state.l2blocks[41];
        let hi = if h > 64 { h - 55 } else { h - 48 } as i32;
        let lo = if l > 64 { l - 55 } else { l - 48 } as i32;
        hi * 16 + lo
    };

    let mut e = FieldEmitter::new(block, c);
    e.text("info_inner_len", 3);
    if len == 40 {
        return;
    }
    e.text("info_type", 2);
    if len == 44 {
        e.text("text02", 2);
        return;
    }

    if infotype == 0x02 {
        e.text("text02", 6);
        e.text("text03", 6);
        e.text("text04", 6);
        e.text("date0", 4);
        e.skip(4);
        e.text("time0", 2);
        e.skip(2);
        e.text("date1", 4);
        e.skip(4);
        e.text("time1", 2);
        e.skip(2);
        e.text("action", 8);
        e.text("cause", 16);
        e.text("subaction", 16);
        e.text("address", 80);
        e.text("text10", 3);
        emit_gps_lat_lon_bcd(&mut e);
        e.text("mapinfo", 58);
        e.text("landmark", 30);
        e.text("direction", 7);
        e.text("distance", 3);
        let n = parse_2digit(&e.c.buf[e.c.pos..]);
        e.text("data_count", 2);
        for _ in 0..n {
            e.text("data", 5);
        }
        return;
    }

    e.hex("vehicle_id", 2);
    if len <= 44 {
        return;
    }

    match infotype {
        0x10 | 0x11 => {
            e.rest_text("text02");
            return;
        }
        0x40 => {
            e.text("text02", 4);
            e.text("text03", 2);
            e.text("text04", 3);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
            e.text("text05", 1);
            e.text("date1_year", 2);
            e.skip(4);
            e.text("time1", 2);
            e.skip(4);
            return;
        }
        0x01 => {
            e.text("text02", 6);
            e.text("text03", 6);
            e.text("text04", 6);
            e.text("date1_year", 2);
            e.skip(4);
            e.text("time1", 2);
            e.skip(4);
            e.text("date2_year", 2);
            e.skip(4);
            e.text("time2", 2);
            e.skip(2);
            e.text("action", 8);
            e.text("cause", 16);
            e.text("subaction", 16);
            e.text("address", 80);
            e.text("text10", 3);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("mapinfo", 58);
            e.text("landmark", 30);
            e.text("direction", 7);
            e.text("distance", 3);
            let n = parse_2digit(&e.c.buf[e.c.pos..]);
            e.text("data_count", 2);
            for _ in 0..n {
                e.text("data", 5);
            }
            return;
        }
        _ => {}
    }

    e.text("date1_year", 2);
    e.skip(4);
    e.text("time1", 2);
    e.skip(4);

    if len <= 56 {
        return;
    }

    match infotype {
        0x69 => {
            e.text("text07", 13);
            e.text("date2_year", 2);
            e.skip(4);
            e.text("time2", 2);
            e.skip(4);
            e.text("date3_year", 2);
            e.skip(4);
            e.text("time3", 2);
            e.skip(4);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
        }
        0x25 => {
            e.text("text07", 1);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
        }
        0x2f | 0x31 => {
            e.rest_text("text07");
        }
        0x32 => {
            let n = (e.peek_u8() - 0x30) as usize;
            e.text("block_count", 1);
            for _ in 0..n {
                e.skip(1 + 2 + 11 + 4 + 2 + 2);
                emit_gps_lat_lon_bcd(&mut e);
                e.skip(2 + 1);
            }
        }
        0x33 => {
            e.text("text07", 61);
            e.text("date2_year", 2);
            e.skip(4);
            e.text("time2", 2);
            e.skip(4);
            e.text("type", 1);
            e.text("address", 40);
            e.text("building", 30);
            e.text("mapinfo", 14);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("landmark", 30);
            e.text("direction", 8);
            e.text("distance", 4);
            e.text("date3_year", 2);
            e.skip(4);
            e.text("time3", 2);
            e.skip(4);
            e.text("date4", 2);
            e.skip(2);
            e.text("time4", 2);
            e.skip(2);
            e.text("name", 30);
            e.text("telephone", 16);
            e.text("text25", 6);
            e.text("text26", 8);
            e.rest_text("text27");
        }
        0x38 => {
            e.text("text07", 63);
            e.text("date2_year", 2);
            e.skip(4);
            e.text("time2", 2);
            e.skip(4);
            e.text("type", 1);
            e.text("address", 40);
            e.text("building", 30);
            e.text("mapinfo", 14);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("landmark", 30);
            e.text("direction", 8);
            e.text("distance", 4);
            e.text("text20", 50);
            e.text("text21", 40);
            e.text("text22", 40);
            e.text("text23", 12);
            e.text("text24", 10);
            e.text("text25", 24);
            e.text("text26", 20);
            e.rest_text("text27");
        }
        0x3d => {
            e.text("text07", 51);
            e.text("message_len", 3);
            e.rest_text("message");
        }
        0x3f => {
            e.text("text07", 5);
            e.text("text08", 4);
            e.text("text09", 12);
            e.text("date_year", 2);
            e.skip(4);
            e.text("body_time", 2);
            e.skip(4);
            e.rest_text("text10");
        }
        0x60 => {
            e.text("text07", 4);
            if len <= 60 {
                return;
            }
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
        }
        0x6d => {
            e.text("text07", 2);
            let len_bytes = e.take(3);
            let n = parse_3digit(len_bytes) as usize;
            e.obj.insert("data_len".to_string(), text_value(len_bytes));
            e.text("data", n);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
        }
        0xa0 => {
            e.text("text07", 4);
            emit_gps_lat_lon_bcd(&mut e);
            e.text("speed", 2);
            e.skip(1);
        }
        _ => {
            e.rest_text("text07");
        }
    }
}
