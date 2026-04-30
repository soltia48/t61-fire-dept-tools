//! Convolutional decoders: R=1/2 K=6 (Layer-2 / RICH / SACCH path)
//! and R=1/2 K=9 (CELP voice path).
//!
//! Both decoders use iterative-deepening backtracking with in-place
//! flips of the input buffer and `(i, state)` resume — recursion
//! resumes from the mismatch point instead of rewinding to offset 0.

use crate::primitives::{crc6, crc16};
use crate::tables::{CONVO_TABLE_26, CONVO_TABLE_29};
use std::sync::OnceLock;

const MAX_EC_26: i32 = 5;
const MAX_EC_29: i32 = 8;

/// Result of a deconvolution attempt.
pub type DecodeResult = Result<(), DecodeError>;

#[derive(Copy, Clone, Debug)]
pub struct DecodeError;

/// Per-(state, symbol) decode table for K=6.  Built once on first use.
///
/// Encoding for entry `[state * 4 + symbol]`:
///   bit 7      : 1 = no valid transition (0xff sentinel)
///   bit 5      : output bit (0 or 1)
///   bits 0..4  : next state
fn decode_step_26() -> &'static [u8; 32 * 4] {
    static TABLE: OnceLock<[u8; 32 * 4]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u8; 32 * 4];
        for s in 0..32u32 {
            let expected = CONVO_TABLE_26[s as usize];
            for v in 0..4u8 {
                t[((s << 2) | v as u32) as usize] = if v == expected {
                    ((s & 0xf) << 1) as u8
                } else if v == (expected ^ 3) {
                    ((((s & 0xf) << 1) | 1) | 0x20) as u8
                } else {
                    0xff
                };
            }
        }
        t
    })
}

/// `t_crc`: 0=none, 1=CRC-6 over `out[..len-5]`, 2=CRC-16 over same.
fn deconvo26_sub(
    input: &mut [u8],
    out: &mut [u8],
    len: usize,
    count: i32,
    max: i32,
    t_crc: i32,
    start_i: usize,
    start_state: u32,
) -> DecodeResult {
    if count > max {
        return Err(DecodeError);
    }
    let table = decode_step_26();
    let mut i = start_i;
    let mut state = start_state;
    while i < len {
        let e = table[((state << 2) | input[i] as u32) as usize];
        if e == 0xff {
            break;
        }
        out[i] = (e >> 5) & 1;
        state = (e & 0x1f) as u32;
        i += 1;
    }
    if i == len {
        if state != 0 {
            return Err(DecodeError);
        }
        if t_crc == 1 && crc6(out, len - 5) != 0 {
            return Err(DecodeError);
        }
        if t_crc == 2 && crc16(out, len - 5) != 0 {
            return Err(DecodeError);
        }
        return Ok(());
    }
    if count == max {
        return Err(DecodeError);
    }
    input[i] ^= 1;
    if deconvo26_sub(input, out, len, count + 1, max, t_crc, i, state).is_ok() {
        return Ok(());
    }
    input[i] ^= 3;
    if deconvo26_sub(input, out, len, count + 1, max, t_crc, i, state).is_ok() {
        return Ok(());
    }
    input[i] ^= 2;
    Err(DecodeError)
}

pub fn deconvo26(input: &mut [u8], out: &mut [u8], len: usize, t_crc: i32) -> DecodeResult {
    (0..MAX_EC_26)
        .find(|&max| deconvo26_sub(input, out, len, 0, max, t_crc, 0, 0).is_ok())
        .map(|_| ())
        .ok_or(DecodeError)
}

fn deconvo29_sub(
    input: &mut [u8],
    out: &mut [u8],
    len: usize,
    count: i32,
    max: i32,
    start_i: usize,
    start_state: u32,
) -> DecodeResult {
    if count > max {
        return Err(DecodeError);
    }
    let mut i = start_i;
    let mut state = start_state;
    while i < len {
        let expected = CONVO_TABLE_29[state as usize];
        if input[i] == expected {
            out[i] = 0;
            state = (state & 0x7f) << 1;
        } else if input[i] == (3 & !expected) {
            out[i] = 1;
            state = ((state & 0x7f) << 1) | 1;
        } else {
            break;
        }
        i += 1;
    }
    if i == len {
        return if state == 0 { Ok(()) } else { Err(DecodeError) };
    }
    if count == max {
        return Err(DecodeError);
    }
    input[i] ^= 1;
    if deconvo29_sub(input, out, len, count + 1, max, i, state).is_ok() {
        return Ok(());
    }
    input[i] ^= 3;
    if deconvo29_sub(input, out, len, count + 1, max, i, state).is_ok() {
        return Ok(());
    }
    input[i] ^= 2;
    Err(DecodeError)
}

pub fn deconvo29(input: &mut [u8], out: &mut [u8], len: usize) -> DecodeResult {
    (0..MAX_EC_29)
        .find(|&max| deconvo29_sub(input, out, len, 0, max, 0, 0).is_ok())
        .map(|_| ())
        .ok_or(DecodeError)
}
