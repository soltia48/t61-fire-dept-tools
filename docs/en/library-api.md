# Library API (`t61_fd` crate)

`t61-fire-dept-tools` ships its slicer and decoder as a Rust library
crate (`t61_fd`) so they can be embedded in other Rust programs. The
two binaries are thin `main()`s on top of the library.

## Cargo entry

```toml
[dependencies]
t61-fire-dept-tools = { path = "..." }   # crate name in Cargo.toml; library name is t61_fd
chrono   = { version = "0.4", default-features = false, features = ["clock"] }
encoding_rs = "0.8"
serde_json = { version = "1", features = ["preserve_order"] }
```

The library targets edition-2024, requiring Rust 1.85+.

## Public surface

```rust
pub use decoder::{Decoder, OutputMode};
pub use slicer::Slicer;
pub use state::{DecoderState, MField, PscState};
pub mod convo;
pub mod gps;
pub mod json;
pub mod primitives;
pub mod slicer;
pub mod state;
pub mod tables;
```

(see [`src/lib.rs`](../../src/lib.rs)).

## Minimal example

```rust
use std::io::stdout;
use t61_fd::{Decoder, OutputMode, Slicer};

fn main() -> std::io::Result<()> {
    let input = std::fs::File::open("frames.t61")?;
    let mut decoder = Decoder::with_mode(stdout().lock(), OutputMode::Json);
    for (i, frame) in Slicer::new(input).enumerate() {
        decoder.process_frame(&frame?, i as u64)?;
    }
    decoder.flush()
}
```

## `Slicer`

```rust
pub struct Slicer<R: Read> { ... }

impl<R: Read> Slicer<R> {
    pub fn new(input: R) -> Self;
    pub fn next_frame(&mut self) -> io::Result<Option<[u8; 48]>>;
}
impl<R: Read> Iterator for Slicer<R> {
    type Item = io::Result<[u8; 48]>;
}
```

- Input is the 4-symbols-per-byte packed format (the same byte stream
  `arib_t61_rx.py --packed-out -` produces).
- `next_frame` yields `Ok(None)` at end of stream; the iterator is
  fused there.
- Read-granularity caveat: short reads from the underlying source
  produce different (but still valid) framing decisions than full
  reads. For deterministic framing on regular-file inputs, use a raw
  file descriptor wrapper.

## `Decoder`

```rust
pub struct Decoder<W: Write> { ... }

impl<W: Write> Decoder<W> {
    pub fn new(out: W) -> Self;                         // OutputMode::Json
    pub fn with_mode(out: W, mode: OutputMode) -> Self;
    pub fn process_frame(&mut self, frame: &[u8; 48], frame_num: u64) -> io::Result<()>;
    pub fn flush(&mut self) -> io::Result<()>;
}

pub enum OutputMode { Json, CelpOnly }
```

Internals:

- `state: DecoderState` — multi-frame buffers (super-frame counter,
  L2 / SACCH block reassembly, RCH / SACCH slots).
- `psc: PscState` — previous-frame TCH and M-field, used to pair-decode
  consecutive frames on the PSC branches.

Each call to `process_frame` builds a `serde_json::Map<String, Value>`
in insertion order, runs the relevant protocol branch, and either
serialises one JSONL line or writes the CELP hex (when
`OutputMode::CelpOnly` and `voice.celp` is present).

## State types

### `MField`

```rust
pub enum MField {
    Idle = 0, Voice = 1, Raw = 2, Data = 3,
    Facch = 4, Free = 5, Busy = 6, Undef = 7,
}

impl MField {
    pub fn from_idx(idx: u8) -> Self;   // 3-bit RICH index
    pub fn name(self) -> &'static str;  // string used in JSONL output
}
```

### `PscState`

```rust
#[derive(Default)]
pub struct PscState {
    pub m: MField,        // previous-frame M-field
    pub tch: [u8; 32],    // previous-frame TCH/FACCH bytes
}
```

Reset to default on `no_signal`, `SS1`, or `unknown_sync` frames; on
PSC frames it carries the previous frame's data so the decoder can
pair-merge `(prev, cur)` for L2 / voice paths.

### `DecoderState`

```rust
pub struct DecoderState {
    pub sacch_count:        usize,        // super-frame counter (0..18)
    pub sacch_buf:          [[u8; 3]; 18],
    pub rch:                [u8; 5],
    pub sacch:              [[u8; 20]; 2],
    pub l2blocks:           [u8; 12 * 64],
    pub l2block_count:      usize,
    pub l2block_last_len:   usize,
    pub sacch_blocks:       [u8; 6 * 64],
    pub sacch_block_count:  usize,
    pub sacch_block_last_len: usize,
}
```

Methods:

| Method | Purpose |
|---|---|
| `new()` / `Default::default()` | zero-init |
| `init_l2block` / `init_sacch_block` | reset block buffer |
| `alloc_l2block(count)` / `alloc_sacch_block(count)` | start a new multi-frame block of `count` slots |
| `assemble_l2block(f_pos, src)` / `assemble_sacch_block(f_pos, src)` | place one slot |
| `reset_idle()` | full idle reset (no_signal / SS1 / unknown sync) |
| `l2block_total_len()` / `sacch_block_total_len()` | concat length |

## Submodule reference

- [`primitives`](../../src/primitives.rs) — `bit_test`, `bit_set`,
  `pack_bits_be`, `parse_2digit`, `parse_3digit`, `Cursor`, `crc6`,
  `crc16`, `interleave`, `slice2`, `compare_sync_byte`,
  `dewhite_psc_tch`, `dewhite_pich`.
- [`convo`](../../src/convo.rs) — `deconvo26`, `deconvo29`,
  `DecodeError`.
- [`gps`](../../src/gps.rs) — `decode_latitude_24`,
  `decode_longitude_24`, `decode_degree_bcd`, `tky_to_wgs84_lat`,
  `tky_to_wgs84_lon`, `gps_status_name`,
  `acch_signal_subcommand_name`, `acch_signal_type_name`.
- [`json`](../../src/json.rs) — `text_value` (Shift_JIS-decoded JSON
  string with NULs stripped), `hex_value`, `FieldEmitter`.
- [`slicer`](../../src/slicer.rs) — `Slicer`, `FRAME_BYTES`.
- [`state`](../../src/state.rs) — `DecoderState`, `PscState`,
  `MField`.
- [`tables`](../../src/tables.rs) — sync words, whitening patterns,
  convolution tables, voice interleave / conversion / normalisation
  tables, compass strings.

## Building from a custom source

Anything that implements `std::io::Read` and yields the
4-symbols-per-byte packed format can drive the slicer; anything that
implements `std::io::Write` can sink the decoder. For a real-time
pipeline, wrap raw file descriptors:

```rust
use std::mem::ManuallyDrop;
use std::os::fd::FromRawFd;

let stdin_fd  = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(0) });
let stdout_fd = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(1) });
let input:  &std::fs::File = &stdin_fd;
let output: &std::fs::File = &stdout_fd;

let mut decoder = t61_fd::Decoder::new(output);
for (i, frame) in t61_fd::Slicer::new(input).enumerate() {
    decoder.process_frame(&frame?, i as u64)?;
}
```

This is exactly what the `t61_frame_slicer` and `t61_fd_decoder`
binaries do — no Rust-side buffering, every frame's output flushes
immediately.
