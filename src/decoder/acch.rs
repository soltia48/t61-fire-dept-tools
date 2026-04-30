//! Layer-2 / RCH / SACCH frame decoding and the ACCH inner-block
//! parser.  The ACCH inner format is shared between Layer-2 (12-byte
//! blocks) and SACCH (6-byte blocks), so it lives here as a single
//! [`emit_acch_fields`] helper consumed by both.

use super::l2_block;
use crate::convo::deconvo26;
use crate::gps::{acch_signal_subcommand_name, acch_signal_type_name};
use crate::json::{hex_value, text_value};
use crate::primitives::{Cursor, bit_set, interleave, slice2};
use crate::state::{DecoderState, MField};
use serde_json::{Map, Value, json};

pub(super) fn proc_layer2(
    record: &mut Map<String, Value>,
    state: &mut DecoderState,
    f1: &[u8; 32],
    f2: &[u8; 32],
    ty: MField,
) {
    let mut layer2 = Map::new();

    let mut frame = [0u8; 32];
    for i in 0..32 {
        frame[i] = (f1[i] & 0xaa) | (f2[i] & 0x55);
    }
    let mut tmp = [0u8; 32];
    interleave(&frame, &mut tmp, 32, 8);
    let mut sliced = [0u8; 128];
    slice2(&tmp, &mut sliced, 128);
    let mut out = [0u8; 128];
    if deconvo26(&mut sliced, &mut out, 128, 2).is_err() {
        layer2.insert("error".to_string(), json!("deconvo_failed"));
        record.insert("layer2".to_string(), Value::Object(layer2));
        state.init_l2block();
        return;
    }

    let mut l2 = [0u8; 1 + 12];
    for i in 0..104 {
        if out[i] != 0 {
            bit_set(&mut l2, i);
        }
    }

    layer2.insert("first".to_string(), json!(out[0] != 0));
    layer2.insert("last".to_string(), json!(out[1] != 0));
    layer2.insert("len_field".to_string(), json!(l2[0] & 0x3f));
    layer2.insert("body".to_string(), hex_value(&l2[1..]));

    match (out[0], out[1]) {
        (1.., 1..) => emit_acch_fields(&mut layer2, &l2),
        (first, last) => {
            if first != 0 {
                state.alloc_l2block((l2[0] & 0x3f) as usize + 1);
            }
            let f_pos = if last != 0 {
                0
            } else {
                (l2[0] & 0x3f) as usize
            };
            let body = if last != 0 {
                &l2[1..1 + (l2[0] & 0x3f) as usize]
            } else {
                &l2[1..1 + 12]
            };
            state.assemble_l2block(f_pos, body);
            if last != 0 {
                l2_block::proc_l2block(&mut layer2, state, ty);
            }
        }
    }

    record.insert("layer2".to_string(), Value::Object(layer2));
}

/// Parse an ACCH command frame from a length-prefixed buffer (1 length
/// byte + body) and insert an `"acch"` sub-object into `parent`.
///
/// Used by both [`proc_layer2`] (12-byte body) and [`proc_sacch`]
/// (6-byte body).
pub(super) fn emit_acch_fields(parent: &mut Map<String, Value>, l2: &[u8]) {
    let mut acch = Map::new();
    let mut c = Cursor::new(l2);
    let mut len = (c.take_u8() & 0x3f) as i32;

    acch.insert("len".to_string(), json!(len));

    if len <= 4 {
        acch.insert("hex".to_string(), hex_value(c.take(len as usize)));
        parent.insert("acch".to_string(), Value::Object(acch));
        return;
    }

    let head = c.peek_u8();
    if head != 1 && head != 2 {
        if head == 4 {
            let command = c.take_u8();
            acch.insert("command".to_string(), json!(command));
            acch.insert("command_str".to_string(), json!("signal"));

            let subcommand = c.take_u8();
            acch.insert("subcommand".to_string(), json!(subcommand));
            if let Some(n) = acch_signal_subcommand_name(subcommand) {
                acch.insert("subcommand_str".to_string(), json!(n));
            }
            if subcommand == 8 {
                acch.insert("hex".to_string(), hex_value(c.take((len - 2) as usize)));
                parent.insert("acch".to_string(), Value::Object(acch));
                return;
            }

            let ty = c.take_u8();
            acch.insert("type".to_string(), json!(ty));
            if let Some(n) = acch_signal_type_name(subcommand, ty) {
                acch.insert("type_str".to_string(), json!(n));
            }
        } else {
            acch.insert("command".to_string(), json!(c.take_u8()));
            acch.insert("hex0".to_string(), hex_value(c.take(2)));
        }
        len -= 3;
    } else {
        acch.insert("command".to_string(), json!(c.take_u8()));
        acch.insert("hex0".to_string(), hex_value(c.take(1)));
        len -= 2;
    }

    let l = c.peek_u8() as i32;
    if l > len {
        acch.insert("hex1".to_string(), hex_value(c.take(len as usize)));
        parent.insert("acch".to_string(), Value::Object(acch));
        return;
    }
    acch.insert("len1".to_string(), json!(c.take_u8()));
    len -= 1;

    if l != 0 {
        acch.insert("hex1".to_string(), hex_value(c.take(l as usize)));
        len -= l;
    }

    let l = c.take_u8() as i32;
    acch.insert("len2".to_string(), json!(l));
    len -= 1;
    if l != 0 {
        acch.insert("hex2".to_string(), hex_value(c.take(l as usize)));
        len -= l;
    }
    if len != 0 {
        acch.insert("text".to_string(), text_value(c.take(len as usize)));
    }

    parent.insert("acch".to_string(), Value::Object(acch));
}

pub(super) fn proc_rch(record: &mut Map<String, Value>, state: &mut DecoderState) {
    const INVALID: [[u8; 5]; 3] = [
        [0x00, 0x00, 0x00, 0x00, 0x00],
        [0x99, 0x91, 0x19, 0x99, 0x00],
        [0x90, 0x01, 0xe9, 0x99, 0x99],
    ];
    if INVALID.contains(&state.rch) {
        return;
    }
    let mut tmp = [0u8; 5];
    let mut sliced = [0u8; 20];
    let mut out = [0u8; 20];
    interleave(&state.rch, &mut tmp, 8, 5);
    slice2(&tmp, &mut sliced, 20);
    if deconvo26(&mut sliced, &mut out, 20, 1).is_err() {
        return;
    }
    let bits: Vec<Value> = out[..8].iter().map(|&b| json!(b)).collect();
    record.insert(
        "rch".to_string(),
        json!({
            "raw": hex_value(&state.rch),
            "bits": Value::Array(bits),
        }),
    );
}

pub(super) fn proc_sacch(
    record: &mut Map<String, Value>,
    state: &mut DecoderState,
    sacch_idx: usize,
) {
    const INVALID: [[u8; 20]; 3] = [
        [0; 20],
        [
            0x99, 0x90, 0x19, 0x99, 0x02, 0x99, 0x90, 0x39, 0x99, 0x04, 0x99, 0x90, 0x59, 0x99,
            0x06, 0x99, 0x90, 0x79, 0x99, 0x08,
        ],
        [
            0x99, 0x90, 0x99, 0x99, 0x0a, 0x99, 0x90, 0xb9, 0x99, 0x0c, 0x99, 0x90, 0xd9, 0x99,
            0x0e, 0x99, 0x90, 0xf9, 0x99, 0x10,
        ],
    ];
    let sacch = state.sacch[sacch_idx];
    if INVALID.contains(&sacch) {
        return;
    }
    let mut tmp = [0u8; 20];
    let mut sliced = [0u8; 80];
    let mut out = [0u8; 80];
    interleave(&sacch, &mut tmp, 16, 10);
    slice2(&tmp, &mut sliced, 80);

    let mut sacch_obj = Map::new();
    if deconvo26(&mut sliced, &mut out, 80, 2).is_err() {
        sacch_obj.insert("error".to_string(), json!("deconvo_failed"));
        record.insert("sacch".to_string(), Value::Object(sacch_obj));
        state.init_sacch_block();
        return;
    }

    let mut l2 = [0u8; 1 + 6];
    for i in 0..56 {
        if out[i] != 0 {
            bit_set(&mut l2, i);
        }
    }
    sacch_obj.insert("raw".to_string(), hex_value(&sacch));
    sacch_obj.insert("first".to_string(), json!(out[0] != 0));
    sacch_obj.insert("last".to_string(), json!(out[1] != 0));
    sacch_obj.insert("len_field".to_string(), json!(l2[0] & 0x3f));
    sacch_obj.insert("body".to_string(), hex_value(&l2[1..]));

    match (out[0], out[1]) {
        (1.., 1..) => {
            emit_acch_fields(&mut sacch_obj, &l2);
            state.init_sacch_block();
        }
        (first, last) => {
            if first != 0 {
                state.alloc_sacch_block((l2[0] & 0x3f) as usize + 1);
            }
            let f_pos = if last != 0 {
                0
            } else {
                (l2[0] & 0x3f) as usize
            };
            let body = if last != 0 {
                &l2[1..1 + (l2[0] & 0x3f) as usize]
            } else {
                &l2[1..1 + 6]
            };
            state.assemble_sacch_block(f_pos, body);
            if last != 0 {
                emit_sacch_block_fields(&mut sacch_obj, state);
            }
        }
    }

    record.insert("sacch".to_string(), Value::Object(sacch_obj));
}

fn emit_sacch_block_fields(parent: &mut Map<String, Value>, state: &DecoderState) {
    let total = state.sacch_block_total_len();
    if total == 0 {
        return;
    }
    let blocks = &state.sacch_blocks[..total];
    let mut c = Cursor::new(blocks);
    let mut len = total as i32;

    let mut block = Map::new();
    block.insert("len".to_string(), json!(total));

    if len <= 4 {
        block.insert("hex".to_string(), hex_value(c.take(len as usize)));
        parent.insert("block".to_string(), Value::Object(block));
        return;
    }

    if c.peek_u8() != 1 {
        block.insert("hex0".to_string(), hex_value(c.take(3)));
        len -= 3;
    } else {
        block.insert("hex0".to_string(), hex_value(c.take(2)));
        len -= 2;
    }

    let l = c.take_u8() as i32;
    block.insert("len1".to_string(), json!(l));
    len -= 1;
    if l != 0 {
        block.insert("hex1".to_string(), hex_value(c.take(l as usize)));
        len -= l;
    }
    let l = c.take_u8() as i32;
    block.insert("len2".to_string(), json!(l));
    len -= 1;
    if l != 0 {
        block.insert("hex2".to_string(), hex_value(c.take(l as usize)));
        len -= l;
    }
    if len != 0 {
        block.insert("text".to_string(), text_value(c.take(len as usize)));
    }

    parent.insert("block".to_string(), Value::Object(block));
}
