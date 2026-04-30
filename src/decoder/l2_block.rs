//! Multi-frame Layer-2 block dispatch.  Selects between FACCH (a
//! short single-frame header) and DATA (which itself splits into
//! text-format / SENDAI / binary-format variants).

use super::{l2_binary, l2_text};
use crate::json::{FieldEmitter, hex_value};
use crate::primitives::Cursor;
use crate::state::{DecoderState, MField};
use serde_json::{Map, Value, json};

pub(super) fn proc_l2block(parent: &mut Map<String, Value>, state: &DecoderState, ty: MField) {
    if state.l2block_count == 0 {
        return;
    }
    let key = match ty {
        MField::Facch => "facch",
        MField::Data => "data",
        _ => "l2block",
    };
    let mut block = Map::new();
    proc_l2block_inner(&mut block, state, ty);
    parent.insert(key.to_string(), Value::Object(block));
}

fn proc_l2block_inner(block: &mut Map<String, Value>, state: &DecoderState, ty: MField) {
    let total = state.l2block_total_len();
    let blocks = &state.l2blocks[..total];
    let mut c = Cursor::new(blocks);

    block.insert("len".to_string(), json!(total));
    block.insert("raw".to_string(), hex_value(blocks));

    match ty {
        MField::Facch => {
            let mut e = FieldEmitter::new(block, &mut c);
            proc_l2block_facch(&mut e, total as i32);
        }
        MField::Data => {
            if (total as i32) < 40 {
                block.insert("data_invalid".to_string(), hex_value(c.rest()));
            } else {
                proc_l2block_data(block, state, &mut c, total as i32);
            }
        }
        _ => {}
    }
}

fn proc_l2block_facch(e: &mut FieldEmitter, mut len: i32) {
    if e.peek_u8() != 1 && e.peek_u8() != 2 {
        e.hex("hex00", 3);
        len -= 3;
    } else {
        e.hex("hex00", 2);
        len -= 2;
    }
    let i = e.peek_u8() as i32;
    e.hex("len01", 1);
    len -= 1;
    if i != 0 {
        e.hex("hex01", i as usize);
        len -= i;
    }
    let i = e.peek_u8() as i32;
    e.hex("len02", 1);
    len -= 1;
    if i != 0 {
        e.hex("hex02", i as usize);
        len -= i;
    }
    if len != 0 {
        e.text("text00", len as usize);
    }
}

/// DATA dispatch: emits the common header fields, then routes to
/// text / SENDAI / binary parsers based on `state.l2blocks[39]` and
/// length heuristics.
fn proc_l2block_data(
    block: &mut Map<String, Value>,
    state: &DecoderState,
    c: &mut Cursor,
    len: i32,
) {
    let info_byte = state.l2blocks[39];
    let textinfo = matches!(info_byte, 0x30..=0x39);

    if textinfo {
        if len >= 42 {
            block.insert("info_type_kind".to_string(), json!("text"));
        }
    } else {
        block.insert("info_type_byte".to_string(), json!(info_byte));
    }

    let mut e = FieldEmitter::new(block, c);
    e.text("text00", 1);
    e.text("from", 4);
    e.skip(4);
    e.text("to", 4);
    e.skip(4);
    e.text("text01", 7);
    e.text("info_len", 3);
    e.text("time", 2);
    e.skip(4);
    e.text("message_id", 4);

    if len < 42 {
        e.rest_text("hex00");
        return;
    }
    drop(e);

    if textinfo {
        l2_text::proc_l2block_data_textinfo(block, state, c, len);
        return;
    }

    if state.l2blocks[(len - 3) as usize] == 3 && matches!(len, 63 | 79 | 84 | 121 | 312) {
        l2_binary::proc_l2block_sendai(block, state, c, len);
        return;
    }

    l2_binary::proc_l2block_binary(block, state, c, len);
}
