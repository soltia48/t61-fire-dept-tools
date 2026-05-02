# t61-fire-dept-tools

Tooling for decoding the Japanese fire-department voice/data channel
defined in **ARIB STD-T61 v1.2 part 2** (SCPC/FDMA, downlink). The
repository ships:

- a Rust crate (`t61_fd`) with two streaming command-line binaries
  (`t61_frame_slicer`, `t61_fd_decoder`)
- a GNU Radio-based SDR receiver in Python (`arib_t61_rx.py`) plus its
  custom π/4-QPSK quasi-coherent demod block (`pi4_qpsk_demod.py`)

The intended capture chain is:

```
SDR / IQ recording
  └─► arib_t61_rx.py        (GNU Radio: RF → π/4-DQPSK demod → 2-bit symbols)
        └─► t61_frame_slicer (sync-word lock → 48-byte FDMA frames)
              └─► t61_fd_decoder (JSONL records, or CELP-only hex)
```

Every stage is pipe-friendly and unbuffered, so a live SDR capture can be
decoded in real time end-to-end.

## Building

Requires a Rust toolchain with edition-2024 support (1.85+).

```sh
cargo build --release
```

The binaries are produced at `target/release/t61_frame_slicer` and
`target/release/t61_fd_decoder`.

## Tools

### `arib_t61_rx.py`

GNU Radio receiver that takes IQ from an SDR, performs frequency
translation, RRC matched filtering, polyphase symbol clock recovery, and
π/4-QPSK quasi-coherent demodulation, and writes 2-bit symbols
downstream. Supported SDR backends (selected with `--device`):

`hackrf`, `rtlsdr`, `airspy`, `airspyhf`, `bladerf`, `uhd` (USRP),
`limesdr`, `plutosdr`, `sdrplay`, `soapy` (generic SoapySDR passthrough).

Per-device defaults (sample rate, decimation, LO offset, gain stages)
are picked automatically; everything is overridable from the CLI.

```sh
# RTL-SDR, on-channel tune, packed 4-symbols/byte to stdout
python3 arib_t61_rx.py --device rtlsdr --freq 467.000e6 --packed-out -

# HackRF with the default 500 kHz LO offset; live decode end-to-end
python3 arib_t61_rx.py -d hackrf -f 467.000e6 --packed-out - \
  | t61_frame_slicer | t61_fd_decoder

# Open the Qt GUI (spectrum / channel waterfall / constellation)
python3 arib_t61_rx.py -d hackrf -f 467.000e6 --gui

# Save decimated baseband IQ for offline replay
python3 arib_t61_rx.py -d airspy -f 467.000e6 --iq-out capture.cf32
```

Output sinks (any combination):

- `--packed-out PATH|-` — 4 symbols per byte, MSB-first (the format
  `t61_frame_slicer` consumes); use `-` to write to stdout
- `--bits-out PATH` — one symbol per byte (raw 0..3), useful for
  inspection
- `--iq-out PATH` — decimated baseband IQ (`complex64`) at the input of
  the matched filter

Tuning knobs worth knowing: `--lo-offset` (defaults to 500 kHz on
HackRF, 0 elsewhere), `--phase-loop-gain` / `--freq-loop-gain` for the
demod PLL, `--timing-loop-bw` / `--timing-max-dev` for the polyphase
clock-sync, and `--squelch-db` / `--fll-bw` for noisy captures.

`pi4_qpsk_demod.py` is the embedded GNU Radio Python block that does
the actual demodulation (JPH06132996A architecture: quasi-coherent with
2nd-order PLL, lock detector with periodic state reset, Gray-coded
MSB-first dibit output). It is imported by `arib_t61_rx.py` and is not
intended to be run standalone.

### `t61_frame_slicer`

Reads a 2-bit-per-symbol byte stream on stdin (each input byte holds four
symbols, MSB first — this is the format produced by
`arib_t61_rx.py --packed-out -`) and writes 48-byte FDMA frames on
stdout.

```sh
t61_frame_slicer < symbols.bin > frames.t61
```

Frame boundaries are recovered by sliding a 32-bit window over the
symbol stream and matching the SS1 / S2 / S6 sync words from ARIB
STD-T61 with a small Hamming-distance tolerance. Frames whose payload
contained no plausible sync are emitted as 48 zero bytes (tagged
`"type": "no_signal"` downstream), preserving the 40 ms super-frame
cadence so downstream timestamps stay aligned with wall-clock time.

### `t61_fd_decoder`

Reads 48-byte FDMA frames on stdin and emits one decoded record per
frame on stdout.

```sh
# default: JSONL
t61_fd_decoder < frames.t61 > frames.jsonl

# CELP-only: write just the 36-character voice payload (one frame per line),
# nothing for non-voice frames. Useful for piping into a CELP player.
t61_fd_decoder -c < frames.t61 > voice.celp
```

Decoded channels:

- **Header** — RICH (R=1/2 K=6 deconvolution + CRC-6), M-field demux
- **PSC TCH/SACCH** — K=9 deconvolution + CRC-16, super-frame counters
- **PICH** — paging channel for SS1 frames
- **Voice** — CELP frame extraction (interleave + voice hash check)
- **L2 / ACCH / FACCH** — Layer-2 multi-frame assembly with text-info
  and binary-info variants, including SENDAI extensions
- **RCH / SACCH** — emitted at the canonical positions (2, 10, 18) of
  the super-frame
- **GPS** — TKY-datum BCD/raw coordinates, converted to WGS84

#### JSONL output

Each line is a single JSON object whose keys preserve insertion order.
Two top-level fields are always present: `frame` (zero-based frame
counter) and `timestamp` (RFC 3339 with millisecond precision, local
timezone — recorded when the frame was decoded). The remaining fields
depend on what the frame contained:

```json
{"frame":0,"timestamp":"2026-04-30T14:21:20.868+09:00","type":"no_signal"}
{"frame":1,"timestamp":"2026-04-30T14:21:20.868+09:00","sync":"S6","error":"rich_deconvo"}
{"frame":42,"timestamp":"2026-04-30T14:21:29.268+09:00","sync":"S2","ftype":"...","m":"voice","voice":{"celp":"..."}}
```

#### CELP-only output

With `-c` / `--celp`, only frames that produced a CELP voice payload
emit anything; each such frame becomes a 36-character lowercase hex
string followed by `\n`. Frames without voice (no_signal, header errors,
voice-hash failures, non-voice M-field) are silent. This keeps the
output suitable for piping straight into a CELP synthesizer without
post-processing.

## Library use

The crate also exposes a library (`t61_fd`) for embedding the slicer and
decoder into other Rust programs:

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

Public re-exports: `Decoder`, `OutputMode`, `Slicer`, `DecoderState`,
`MField`, `PscState`. The submodules `convo`, `gps`, `json`,
`primitives`, `slicer`, `state`, and `tables` are exposed for users who
want to reuse pieces of the pipeline directly.

## Layout

```
arib_t61_rx.py          # GNU Radio SDR receiver (multi-device front-end)
pi4_qpsk_demod.py       # custom π/4-QPSK quasi-coherent demod block
src/
├── lib.rs              # public re-exports
├── bin/
│   ├── t61_frame_slicer.rs
│   └── t61_fd_decoder.rs
├── slicer.rs           # 2-bit-symbol stream → 48-byte frames
├── decoder/            # 48-byte frames → JSONL / CELP
│   ├── mod.rs          # Decoder + OutputMode + frame branching
│   ├── header.rs       # RICH / PSC TCH+SACCH / PICH
│   ├── voice.rs        # CELP voice frame
│   ├── acch.rs         # Layer-2 / RCH / SACCH / ACCH inner fields
│   ├── l2_block.rs     # L2 multi-frame block dispatch + FACCH
│   ├── l2_text.rs      # text-format L2 data variants
│   ├── l2_binary.rs    # binary-format + SENDAI L2 data variants
│   └── gps_emit.rs     # GPS sub-object emitters
├── convo.rs            # K=6 / K=9 convolutional decoders
├── primitives.rs       # bit ops, CRC-6/16, sync match, dewhitening
├── json.rs             # FieldEmitter (Cursor + serde_json::Map DSL)
├── state.rs            # DecoderState, PscState, MField
├── gps.rs              # TKY → WGS84, lat/lon/speed/dir parsers
└── tables.rs           # sync words, whitening / interleave tables, ...
```

## Real-time pipe usage

Both Rust binaries access stdin/stdout as raw file descriptors (no
Rust-side buffering), and `arib_t61_rx.py` flushes its file-descriptor
sink unbuffered when `--packed-out -` is used, so each frame's output
reaches the next stage as soon as it is produced. A typical live decode
looks like:

```sh
PYTHONUNBUFFERED=1 python3 arib_t61_rx.py -d hackrf -f 467.000e6 --packed-out - \
  | t61_frame_slicer | t61_fd_decoder
```
