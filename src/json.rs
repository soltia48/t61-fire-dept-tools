//! Helpers for building serde_json [`Value`]s and a [`FieldEmitter`]
//! DSL that pairs a [`Cursor`] with an object builder.
//!
//! With `serde_json`'s `preserve_order` feature enabled, all `Map`s in
//! this crate are insertion-ordered, so emitted records read in the
//! same logical sequence as the C reference output.

use crate::primitives::Cursor;
use serde_json::{Map, Value};

/// Bytes → JSON string with NUL bytes silently dropped (matches the
/// original C decoder's text-field convention).  Bytes are decoded as
/// Shift_JIS; invalid sequences are replaced with U+FFFD.
pub fn text_value(bytes: &[u8]) -> Value {
    let cleaned: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
    let (decoded, _, _) = encoding_rs::SHIFT_JIS.decode(&cleaned);
    Value::String(decoded.into_owned())
}

/// Bytes → lowercase hex string.
pub fn hex_value(bytes: &[u8]) -> Value {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{:02x}", b).unwrap();
    }
    Value::String(s)
}

/// Pairs a [`Cursor`] with a JSON object builder for compact field-by
/// -field record building.  Used by the long parser variants in
/// `proc_l2block_data_textinfo` and friends.
pub struct FieldEmitter<'a, 'buf> {
    pub obj: &'a mut Map<String, Value>,
    pub c: &'a mut Cursor<'buf>,
}

impl<'a, 'buf> FieldEmitter<'a, 'buf> {
    pub fn new(obj: &'a mut Map<String, Value>, c: &'a mut Cursor<'buf>) -> Self {
        FieldEmitter { obj, c }
    }

    pub fn insert(&mut self, name: &str, value: impl Into<Value>) {
        self.obj.insert(name.to_string(), value.into());
    }

    pub fn text(&mut self, name: &str, n: usize) {
        self.obj
            .insert(name.to_string(), text_value(self.c.take(n)));
    }

    pub fn rest_text(&mut self, name: &str) {
        self.obj.insert(name.to_string(), text_value(self.c.rest()));
    }

    pub fn hex(&mut self, name: &str, n: usize) {
        self.obj.insert(name.to_string(), hex_value(self.c.take(n)));
    }

    pub fn rest_hex(&mut self, name: &str) {
        self.obj.insert(name.to_string(), hex_value(self.c.rest()));
    }

    pub fn skip(&mut self, n: usize) {
        self.c.skip(n);
    }

    pub fn peek_u8(&self) -> u8 {
        self.c.peek_u8()
    }

    pub fn take_u8(&mut self) -> u8 {
        self.c.take_u8()
    }

    pub fn take(&mut self, n: usize) -> &'buf [u8] {
        self.c.take(n)
    }
}
