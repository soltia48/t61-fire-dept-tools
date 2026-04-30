//! T61 frame header decoding: RICH, PSC TCH/SACCH, and PICH.

use crate::convo::deconvo26;
use crate::json::hex_value;
use crate::primitives::{interleave, pack_bits_be, slice2};
use crate::state::{DecoderState, MField};
use serde_json::{Map, Value, json};

/// Decode the RICH header (deinterleave + slice + deconvolve), insert
/// `rich`, `mfield`, and optionally `sync_acquired` into `record`.
/// Returns the M-field on success.
pub(super) fn proc_t61_frame(
    record: &mut Map<String, Value>,
    ftype: u32,
    in_buf: &[u8; 7],
) -> Option<MField> {
    let mut d = [0u8; 7];
    let mut sliced = [0u8; 28];
    let mut r = [0u8; 28];

    interleave(in_buf, &mut d, 8, 7);
    slice2(&d, &mut sliced, 28);

    if deconvo26(&mut sliced, &mut r, 28, 1).is_err() {
        record.insert("error".to_string(), json!("rich_deconvo"));
        return None;
    }

    record.insert(
        "rich".to_string(),
        json!([
            (r[0] << 1) | r[1],
            (r[2] << 2) | (r[3] << 1) | r[4],
            (r[5] << 2) | (r[6] << 1) | r[7],
            (r[8] << 2) | (r[9] << 1) | r[10],
            (r[11] << 2) | (r[12] << 1) | r[13],
            (r[14] << 2) | (r[15] << 1) | r[16],
        ]),
    );

    let idx = (r[5] << 2) | (r[6] << 1) | r[7];
    let m = MField::from_idx(idx);
    record.insert("mfield".to_string(), json!(m.name()));
    if idx == 0 && ftype == 0x2f94d06b {
        record.insert("sync_acquired".to_string(), json!(true));
    }
    Some(m)
}

pub(super) fn proc_psc_tch(record: &mut Map<String, Value>, buf: &[u8; 35]) {
    record.insert("tch".to_string(), hex_value(&buf[..32]));
}

fn sacch_unpack_pair(a: &[u8; 3], b: &[u8; 3], out: &mut [u8; 5]) {
    out[0] = (a[0] << 4) | (a[1] >> 4);
    out[1] = (a[1] << 4) | (a[2] >> 4);
    out[2] = (a[2] << 4) | b[0];
    out[3] = b[1];
    out[4] = b[2];
}

pub(super) fn proc_psc_sacch(
    record: &mut Map<String, Value>,
    state: &mut DecoderState,
    buf: &[u8; 35],
    top_frame: bool,
) {
    record.insert(
        "sacch_slot".to_string(),
        json!({
            "rch_nib": buf[32] & 0xf,
            "data16": ((buf[33] as u32) << 8) | buf[34] as u32,
        }),
    );

    if top_frame {
        if state.sacch_count != 0 {
            state.sacch_count = 0;
        }
    } else if state.sacch_count == 0 {
        return;
    }

    state.sacch_buf[state.sacch_count] = [buf[32] & 0xf, buf[33], buf[34]];
    state.sacch_count += 1;

    match state.sacch_count {
        2 => sacch_unpack_pair(&state.sacch_buf[0], &state.sacch_buf[1], &mut state.rch),
        10 => {
            for k in 0..4 {
                let mut tmp = [0u8; 5];
                sacch_unpack_pair(
                    &state.sacch_buf[2 + 2 * k],
                    &state.sacch_buf[2 + 2 * k + 1],
                    &mut tmp,
                );
                state.sacch[0][5 * k..5 * k + 5].copy_from_slice(&tmp);
            }
        }
        18 => {
            for k in 0..4 {
                let mut tmp = [0u8; 5];
                sacch_unpack_pair(
                    &state.sacch_buf[10 + 2 * k],
                    &state.sacch_buf[10 + 2 * k + 1],
                    &mut tmp,
                );
                state.sacch[1][5 * k..5 * k + 5].copy_from_slice(&tmp);
            }
        }
        _ => {}
    }
}

pub(super) fn proc_pich(record: &mut Map<String, Value>, pich: &[u8; 35]) {
    let mut tmp = [0u8; 13];
    let mut sliced = [0u8; 52];
    let mut bits = [0u8; 52];

    interleave(&pich[..13], &mut tmp, 13, 8);
    slice2(&tmp, &mut sliced, 52);
    if deconvo26(&mut sliced, &mut bits, 52, 1).is_err() {
        record.insert("error".to_string(), json!("pich_deconvo"));
        return;
    }

    record.insert(
        "pich".to_string(),
        json!({
            "flag": bits[0],
            "group": bits[1],
            "a": pack_bits_be(&bits[2..5]),
            "b": pack_bits_be(&bits[5..8]),
            "c": pack_bits_be(&bits[8..11]),
            "slot": pack_bits_be(&bits[11..16]),
            "firedep": pack_bits_be(&bits[16..28]),
            "station": pack_bits_be(&bits[28..40]),
            "flag2": bits[40],
        }),
    );
}
