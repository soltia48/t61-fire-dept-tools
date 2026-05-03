# Frame slicer (`t61_frame_slicer`)

The slicer's job is to convert a freewheeling 2-bit-symbol stream into
a stream of aligned 48-byte FDMA frames. It owns no protocol
knowledge beyond the three ARIB sync words and the canonical sync-word
position in a frame.

## I/O

```
stdin   (4 symbols / byte, MSB-first)  —  the format `--packed-out -` produces
stdout  (48 bytes / frame)             —  what `t61_fd_decoder` consumes
```

Both descriptors are accessed with `from_raw_fd(0)` /
`from_raw_fd(1)` so reads/writes hit `read(2)` / `write(2)` directly,
without any Rust-side buffering. This keeps the live SDR pipeline
real-time. See [t61_frame_slicer.rs:21-30](../../src/bin/t61_frame_slicer.rs#L21-L30).

## Frame layout

| Offset (sym) | Length (sym) | Field |
|---|---|---|
| 0 | 92 | data leading to the sync word |
| 92 | 16 | sync window (longest sync word, SS1) |
| 108 | 84 | data after the sync word |
| 192 | — | end of frame |

In packed bytes, that's 48 bytes per frame (`192 / 4`).

## Sync words

ARIB STD-T61 v1.2 part 2 defines three sync words:

| Sync | Symbols (MSB pair first) | Packed | Used for |
|---|---|---|---|
| SS1 | `0,2,3,3,2,1,1,0,3,1,0,0,1,2,2,3` | `0x2f94d06b` | center-to-terminal sync |
| S2  | `2,1,3,1,0,2,0,3,1,2`             | `0x9d236xxx` | top of super-frame |
| S6  | `0,1,3,2,1,1,1,2,3,3`             | `0x1e56fxxx` | mid super-frame |

S2 and S6 are 10-symbol words shorter than the SS1 window. They are
matched with a top-aligned 32-bit pattern + 20-bit mask
(`0xfffff000`), so the same sliding-window comparator handles all
three. See [tables.rs](../../src/tables.rs) and
[slicer.rs:53-69](../../src/slicer.rs#L53-L69).

## Search algorithm

```
const SYNC_WORD_OFFSET     = 92    // expected sync position, symbols
const SYNC_WINDOW_SYMBOLS  = 16    // size of the sliding window
const LP_R_FLUCT           = 4     // expected drift
const ERROR_MAX            = 1     // tolerated symbol errors
```

For every candidate frame:

1. Build a 32-bit sliding window over the symbol buffer starting at
   `SYNC_WORD_OFFSET`.
2. **Fast path**: try the most likely position (`LP_R_FLUCT` = 4
   symbols later) first.
3. Otherwise scan forward up to `valid - 192` symbols and compare each
   window against every entry in `PACKED_SW_TAB` with the entry's
   pattern + mask, allowing up to `ERROR_MAX` 2-bit-symbol errors
   (Hamming-style: count of non-zero `(window ^ pattern) & mask`
   bits).
4. On a match, patch the canonical pattern back into the symbol
   buffer (cleans up tolerated errors before the decoder sees them).

The slicer keeps a buffer of three frames' worth of symbols. After
emitting a frame it slides forward by `FRAME_SYMBOLS - LP_R_FLUCT`
(188 symbols) so the next sync search can absorb up to ±4 symbols of
drift without losing alignment.

## No-signal handling

When sync cannot be located in the current sliding window, the slicer
emits a 48-byte all-zero "no_signal" placeholder (`NO_SIGNAL_FRAME`)
and slides the buffer forward by one frame's worth of symbols. The
decoder downstream sees byte 0x17 == 0 and labels the record
`"type":"no_signal"`. This:

- Preserves the 40 ms super-frame cadence so timestamps stay
  wall-clock aligned.
- Keeps the decoder's `frame` counter in lockstep with real-world
  time.

A trailing no_signal is also emitted at EOF if the buffer still holds
≥ 10 symbols (less than a full frame but enough for sync match
ambiguity).

## Pending-frame mechanism

If the matched sync position is *farther* into the buffer than the
expected drift (`ret > FRAME_SYMBOLS - LP_R_FLUCT`), the slicer
inserts a no_signal frame *before* the just-extracted real frame.
The real frame is held in a one-slot `pending` queue and emitted on
the next call to `next_frame`. This way frame numbering stays exactly
aligned with what the decoder expects (one slot lost = one no_signal,
no shift). See
[slicer.rs:236-247](../../src/slicer.rs#L236-L247).

## Read-granularity contract

Because the search is greedy and re-syncs only on match-or-slide,
**short reads from the underlying source produce different (but still
valid) framing decisions than full reads**. For deterministic framing
on regular-file inputs, supply a `Read` that returns full chunks
(e.g. a raw file descriptor wrapper rather than `io::stdin()`'s
buffered `StdinLock`). Live pipe consumers don't need to care:
once the stream is synced the framing is the same regardless of read
granularity.

## Library use

```rust
use t61_fd::Slicer;

let f = std::fs::File::open("symbols.bin")?;
for frame in Slicer::new(f) {
    let frame: [u8; 48] = frame?;
    // ...
}
```

`Slicer` implements `Iterator<Item = io::Result<[u8; 48]>>` and is
fused at EOF.
