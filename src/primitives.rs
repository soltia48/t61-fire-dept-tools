//! Stateless byte-level utilities: bit ops, slice cursor, parsers,
//! CRC, and frame slicing/interleaving primitives.

use crate::tables::{WP_PCC_TCH, WP_PSC_TCH};

// -------- bit ops --------------------------------------------------

/// Test bit `n` of an MSB-first packed bit array.
#[inline]
pub fn bit_test(x: &[u8], n: usize) -> bool {
    (x[n / 8] & (0x80 >> (n % 8))) != 0
}

/// Set bit `n` of an MSB-first packed bit array.
#[inline]
pub fn bit_set(x: &mut [u8], n: usize) {
    x[n / 8] |= 0x80 >> (n % 8);
}

/// Pack a slice of single-bit values (0 or 1, MSB-first) into a u32.
pub fn pack_bits_be(bits: &[u8]) -> u32 {
    bits.iter().fold(0u32, |acc, &b| (acc << 1) | b as u32)
}

// -------- ASCII-decimal parsers ------------------------------------

/// 2-byte ASCII decimal "NN" -> integer.
pub fn parse_2digit(b: &[u8]) -> u32 {
    (b[0] - b'0') as u32 * 10 + (b[1] - b'0') as u32
}

/// 3-byte ASCII decimal "NNN" -> integer.
pub fn parse_3digit(b: &[u8]) -> u32 {
    (b[0] - b'0') as u32 * 100 + (b[1] - b'0') as u32 * 10 + (b[2] - b'0') as u32
}

// -------- Cursor ---------------------------------------------------

/// Slice walker.  All `take*` methods advance an internal position.
/// Lifetimes are tied to the underlying buffer so taken slices outlive
/// `&mut self`, allowing them to be passed to side-effecting methods.
pub struct Cursor<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }
    pub fn take_u8(&mut self) -> u8 {
        let b = self.buf[self.pos];
        self.pos += 1;
        b
    }
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }
    pub fn peek_u8(&self) -> u8 {
        self.buf[self.pos]
    }
    pub fn peek_at(&self, offset: usize) -> u8 {
        self.buf[self.pos + offset]
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
}

// -------- CRCs (one bit per input byte) ----------------------------

/// CRC-6 (1 + X + X^6) over an unpacked-bit input stream.
pub fn crc6(dat: &[u8], len: usize) -> u8 {
    let mut crc: u8 = 0x3f;
    for &d in &dat[..len] {
        crc <<= 1;
        let bit_in = d ^ ((crc & 0x40 != 0) as u8);
        if bit_in != 0 {
            crc ^= 0x3;
        }
    }
    crc & 0x3f
}

/// CRC-16 (1 + X^5 + X^12 + X^16) over an unpacked-bit input stream.
pub fn crc16(dat: &[u8], len: usize) -> u16 {
    let mut crc: u32 = 0xffff;
    for &d in &dat[..len] {
        crc <<= 1;
        let bit_in = d ^ ((crc & 0x10000 != 0) as u8);
        if bit_in != 0 {
            crc ^= 0x1021;
        }
    }
    (crc & 0xffff) as u16
}

// -------- frame primitives -----------------------------------------

/// Generic interleave/deinterleave between an x-by-y bit grid.
pub fn interleave(org: &[u8], dst: &mut [u8], x: usize, y: usize) {
    let bytes = x * y / 8;
    dst[..bytes].fill(0);
    for j in 0..y {
        for i in 0..x {
            if bit_test(org, j * x + i) {
                bit_set(dst, j + y * i);
            }
        }
    }
}

/// Slice each input byte into four 2-bit symbols (MSB pair first).
pub fn slice2(input: &[u8], out: &mut [u8], len: usize) {
    for k in 0..len {
        let shift = 6 - (k % 4) * 2;
        out[k] = (input[k / 4] >> shift) & 3;
    }
}

/// Match a packed-byte sync window against a 2-bit-per-symbol pattern,
/// allowing up to `max_err` symbol errors.
pub fn compare_sync_byte(b: &[u8], sw: &[u8], max_err: u32) -> bool {
    let mut errcnt = 0;
    for (i, &expected) in sw.iter().enumerate() {
        let shift = 6 - (i % 4) * 2;
        let actual = (b[i / 4] >> shift) & 3;
        if actual != expected {
            errcnt += 1;
        }
    }
    errcnt <= max_err
}

// -------- whitening ------------------------------------------------

/// Dewhiten a PSC TCH/SACCH frame.  Output layout:
///   `out[0..32]`  = TCH/FACCH
///   `out[32..35]` = SACCH/RCH
pub fn dewhite_psc_tch(frame: &[u8], out: &mut [u8; 35]) {
    let w = &WP_PSC_TCH;
    for i in 0..12 {
        out[i] = frame[4 + i] ^ w[i];
    }
    for i in 0..3 {
        out[32 + i] = frame[25 + i] ^ w[12 + i];
    }
    for i in 0..20 {
        out[12 + i] = frame[28 + i] ^ w[15 + i];
    }
}

/// Dewhiten a PICH frame into `out[0..13]`.
pub fn dewhite_pich(frame: &[u8], out: &mut [u8; 35]) {
    for i in 0..13 {
        out[i] = frame[34 + i] ^ WP_PCC_TCH[i];
    }
}
