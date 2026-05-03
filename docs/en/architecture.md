# Architecture overview

`t61-fire-dept-tools` decodes the **ARIB STD-T61 v1.2 part 2** SCPC/FDMA
downlink used by Japanese fire-department radio dispatch. The repo is a
three-stage Unix pipeline whose stages can run independently or stream
end-to-end in real time.

```
┌──────────────┐    cf32     ┌──────────────────┐  4 sym/byte ┌─────────────────┐  48 B/frame ┌──────────────────┐  JSONL / CELP
│ SDR (HackRF, │───────────▶│ arib_t61_rx.py   │────────────▶│ t61_frame_slicer│────────────▶│ t61_fd_decoder   │──────────────▶
│ RTL-SDR …)   │            │ (GNU Radio + π/4 │             │ (Rust)          │             │ (Rust)           │
└──────────────┘            │  QPSK demod)     │             └─────────────────┘             └──────────────────┘
                            └──────────────────┘
```

## Stage responsibilities

### 1. SDR + GNU Radio receiver (Python)

[`arib_t61_rx.py`](../../arib_t61_rx.py) uses `gr-osmosdr` to talk to
any of ten SDR backends (HackRF, RTL-SDR, Airspy R2/Mini, Airspy HF+,
BladeRF, USRP, LimeSDR, PlutoSDR, SDRplay, generic Soapy). It performs:

1. RF tuning (with optional LO offset to dodge DC artifacts)
2. Frequency translation + low-pass + decimation to ~4 sps
3. Optional power squelch + feedforward AGC
4. Root-raised-cosine matched filtering (β = 0.2)
5. Optional FLL band-edge frequency correction
6. Polyphase symbol clock recovery (4 sps → 1 sps)
7. π/4-shifted QPSK quasi-coherent demodulation
   ([`pi4_qpsk_demod.py`](../../pi4_qpsk_demod.py))
8. 4-symbol-per-byte packing for downstream consumption

The output is a stream of 2-bit symbols (Gray-coded, MSB first) at the
nominal 4800 baud symbol rate. Symbols can be written to file, packed
into bytes, or piped straight to the next stage via a file-descriptor
sink.

### 2. Frame slicer (Rust)

[`t61_frame_slicer`](../../src/bin/t61_frame_slicer.rs) reads the
packed-symbol stream and slides a 32-bit window over it looking for the
ARIB sync words **SS1**, **S2**, and **S6** at the canonical position
within a 192-symbol frame. Each correctly-aligned frame is emitted as
exactly 48 bytes (192 symbols × 2 bits / 8). When sync cannot be
re-acquired within one frame's worth of symbols, an all-zero
"placeholder" frame is emitted instead so downstream timestamping stays
locked to the 40 ms super-frame cadence.

### 3. Decoder (Rust)

[`t61_fd_decoder`](../../src/bin/t61_fd_decoder.rs) consumes 48-byte
frames and emits one record per frame:

- **`OutputMode::Json`** (default) — one JSON object per line (JSONL),
  with insertion-ordered keys preserved by `serde_json`'s
  `preserve_order` feature.
- **`OutputMode::CelpOnly`** (`-c` / `--celp`) — only the 36-character
  hex CELP payload, one per voice frame, suitable for piping into a
  CELP synthesizer.

The decoder runs the full ARIB stack: dewhitening, deinterleaving,
convolutional decoding (R = 1/2 K = 6 for control channels, K = 9 for
voice), CRC checks, and protocol-specific parsers for header (RICH /
M-field), PSC TCH, SACCH/RCH, PICH paging, Layer-2 multi-frame
reassembly, ACCH commands, GPS, FACCH, and a SENDAI-region binary
extension.

## Why three independent stages?

- **Real-time**: the Rust binaries access stdin/stdout as raw file
  descriptors (`from_raw_fd(0)` / `from_raw_fd(1)`) so reads and
  writes go straight to `read(2)` / `write(2)` syscalls with no
  Rust-side buffering. `arib_t61_rx.py` uses
  `blocks.file_descriptor_sink(... 1)` (stdout) with
  `set_unbuffered(True)` on plain file sinks. Combined, a single live
  pipeline runs end-to-end without ever stalling on buffer fills.
- **Replayability**: any intermediate format (cf32 IQ, 1-symbol-per-byte
  bits, packed symbols, 48-byte frames) can be saved and replayed
  later for analysis or regression testing.
- **Reproducibility**: the same Rust binary that decodes a live SDR
  feed can decode a captured `*.t61` file deterministically — the
  output for a given input file is stable across runs.
- **Composability**: anything that produces 4-symbols-per-byte packed
  bytes (e.g. another decoder, a test vector generator) can feed
  `t61_frame_slicer`. Anything that produces 48-byte frames can feed
  `t61_fd_decoder`.

## Source layout

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

## Build & run

```sh
# Rust 1.85+ (edition 2024)
cargo build --release

# live decode (HackRF, default 500 kHz LO offset)
PYTHONUNBUFFERED=1 python3 arib_t61_rx.py -d hackrf -f 274.60625e6 --packed-out - \
  | target/release/t61_frame_slicer \
  | target/release/t61_fd_decoder
```

Read on:

- [SDR receiver pipeline](sdr-pipeline.md) for the GNU Radio flowgraph
  and PLL details
- [Frame slicer](frame-slicer.md) for the sync-word search
- [Frame decoder](decoder.md) for the protocol stack
- [JSONL / CELP output](json-output.md) for the on-the-wire output
  schema
- [Library API](library-api.md) for embedding the decoder
