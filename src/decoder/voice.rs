//! CELP voice frame decoding.
//!
//! Two consecutive 32-byte TCH frames are paired, K=9 R=1/2
//! deconvolved, the embedded hash is checked, and the result is
//! permuted via `CELP_CONV_TABLE`, normalised, and emitted as a
//! 38-hex-char "celp" string (last nibble carries the hash flag).

use crate::convo::deconvo29;
use crate::primitives::{bit_set, bit_test};
use crate::tables::{
    CELP_CONV_TABLE, CELP_NORMALIZE, VOICE_CONV_MATRIX, VOICE_INTERLEAVE_MATRIX, VOICE_MAGIC_TABLE,
    VOICE_TAIL_MASK,
};
use serde_json::{Map, Value, json};

const VOICE_HASH_MAGIC: i32 = 0x327;

fn calc_voice_hash(blk: &[u8]) -> i32 {
    let mut r: i32 = 0;
    let mut p = 88usize;
    for _ in 0..9 {
        r = (r << 1) | (blk[p] & 1) as i32;
        p = p.wrapping_sub(1);
    }
    for _ in 0..9 {
        r = (r << 1) | (blk[p] & 1) as i32;
        p = p.wrapping_sub(1);
        if r & (1 << 9) != 0 {
            r ^= VOICE_HASH_MAGIC;
        }
    }
    let mut p = 27usize;
    for _ in 0..23 {
        r = (r << 1) | (blk[p] & 1) as i32;
        p = p.wrapping_sub(1);
        if r & (1 << 9) != 0 {
            r ^= VOICE_HASH_MAGIC;
        }
    }
    for _ in 0..9 {
        r <<= 1;
        if r & (1 << 9) != 0 {
            r ^= VOICE_HASH_MAGIC;
        }
    }
    r & 0x1ff
}

/// True when the embedded hash mismatches the recomputed one.
fn check_voice_hash(blk: &[u8]) -> bool {
    let s = calc_voice_hash(blk);
    let mut r: i32 = 0;
    for i in 0..5 {
        r = (r << 1) | (blk[i] & 1) as i32;
    }
    for i in 0..4 {
        r = (r << 1) | (blk[89 + i] & 1) as i32;
    }
    (r ^ s) != 0
}

pub(super) fn proc_voice(record: &mut Map<String, Value>, f1: &[u8; 32], f2: &[u8; 32]) {
    let mut voice = Map::new();

    // blank-frame check: all 0x00 or all 0xff in either half
    let blank1 = f1[..16].iter().all(|&b| b == 0 || b == 0xff);
    let blank2 = f2[16..].iter().all(|&b| b == 0 || b == 0xff);
    if blank1 || blank2 {
        voice.insert("error".to_string(), json!("blank"));
        record.insert("voice".to_string(), Value::Object(voice));
        return;
    }

    let mut frame = [0u8; 32];
    frame[..16].copy_from_slice(&f1[..16]);
    frame[16..].copy_from_slice(&f2[16..]);
    let mut v_tmp = [0u8; 32];

    for i in 0..256 {
        if bit_test(&frame, VOICE_INTERLEAVE_MATRIX[i] as usize) {
            bit_set(&mut v_tmp, i);
        }
    }

    let mut deconvo_input = [0u8; 101];
    for i in 0..25 {
        deconvo_input[i * 4] = (v_tmp[i] >> 6) & 3;
        deconvo_input[i * 4 + 1] = (v_tmp[i] >> 4) & 3;
        deconvo_input[i * 4 + 2] = (v_tmp[i] >> 2) & 3;
        deconvo_input[i * 4 + 3] = v_tmp[i] & 3;
    }
    deconvo_input[100] = (v_tmp[25] >> 6) & 3;

    for i in 0..101 {
        deconvo_input[i] =
            VOICE_CONV_MATRIX[VOICE_MAGIC_TABLE[i] as usize][deconvo_input[i] as usize];
    }

    let mut deconvo_result = [0u8; 101 + 54 + 1];
    if deconvo29(&mut deconvo_input, &mut deconvo_result, 101).is_err() {
        voice.insert("error".to_string(), json!("deconvo_failed"));
        record.insert("voice".to_string(), Value::Object(voice));
        return;
    }

    for i in 0..7 {
        v_tmp[i + 25] ^= VOICE_TAIL_MASK[i];
    }
    for i in 0..54 {
        deconvo_result[i + 101] = bit_test(&v_tmp, i + 202) as u8;
    }

    let hash_mismatch = check_voice_hash(&deconvo_result);
    deconvo_result[155] = hash_mismatch as u8;
    voice.insert("hash_ok".to_string(), json!(!hash_mismatch));

    // MCA reordering: reverse-copy two non-contiguous spans.
    let mut celp_mca = [0u8; 139];
    let mut j = 0usize;
    for i in (5..=88).rev() {
        celp_mca[j] = deconvo_result[i];
        j += 1;
    }
    for i in (101..=154).rev() {
        celp_mca[j] = deconvo_result[i];
        j += 1;
    }
    celp_mca[j] = deconvo_result[155];

    let mut celp_raw = [0u8; 18];
    for i in 0..139 {
        if celp_mca[CELP_CONV_TABLE[i] as usize] != 0 {
            bit_set(&mut celp_raw, i);
        }
    }
    for i in 0..18 {
        celp_raw[i] ^= CELP_NORMALIZE[i];
    }

    // 38 hex chars: 17 full bytes + final 2 nibbles.
    use std::fmt::Write;
    let mut s = String::with_capacity(38);
    for i in 0..17 {
        write!(s, "{:02x}", celp_raw[i]).unwrap();
    }
    write!(s, "{:x}{:x}", celp_raw[17] >> 4, celp_mca[138]).unwrap();
    voice.insert("celp".to_string(), Value::String(s));

    record.insert("voice".to_string(), Value::Object(voice));
}
