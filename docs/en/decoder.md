# Frame decoder (`t61_fd_decoder`)

The decoder consumes 48-byte FDMA frames and emits one record per
frame, either as JSONL (default) or as a CELP-only hex stream
(`-c` / `--celp`). Internally it runs the full ARIB protocol stack
on every frame: dewhitening, deinterleaving, convolutional decoding,
CRC checks, and protocol-specific parsing for header/PSC/PICH/Layer-2/
voice/SACCH/RCH/GPS.

## Frame dispatch

[`Decoder::process_frame`](../../src/decoder/mod.rs#L83-L123) inspects
byte `frame[0x17]` and the four candidate sync words to pick a
branch:

| Condition | Branch | Notes |
|---|---|---|
| `frame[0x17] == 0` | `no_signal` | Slicer placeholder; reset PSC + idle. |
| `compare_sync_byte(... S6)` | PSC (`S6`) | Mid super-frame. |
| `compare_sync_byte(... S2)` | PSC (`S2`) | Top of super-frame; resets `sacch_count`. |
| `compare_sync_byte(... SS1)` | SS1 | Center-to-terminal sync; PICH paging. |
| otherwise | `unknown_sync` | reset PSC + idle. |

Sync-word match here uses up to **3** symbol errors
(`MAX_SW_ERR = 3`), looser than the slicer's ±1, because the slicer
has already patched the canonical pattern back in.

After the branch finishes, [`finalize_super_frame`](../../src/decoder/mod.rs#L140-L151)
emits RCH at frame 2, SACCH[0] at frame 10, and SACCH[1] at frame 18
within each super-frame, then resets `sacch_count`.

## Header path

`proc_t61_frame` ([header.rs:11-48](../../src/decoder/header.rs#L11-L48))
operates on bytes `0x10..0x17` of the frame:

1. **Deinterleave** the 7-byte block as an 8×7 bit grid
   ([`primitives::interleave`](../../src/primitives.rs#L107-L118)).
2. **Slice** each byte into four 2-bit symbols (`slice2`).
3. **Deconvolve** with the K=6 R=1/2 decoder
   ([`convo::deconvo26`](../../src/convo.rs#L100-L105), CRC-6 check).
4. Failure → emit `"error":"rich_deconvo"`.
5. Otherwise emit:

   - `"rich"` — six 2- or 3-bit RICH fields packed back into integers.
   - `"mfield"` — string name from `r[5..7]`
     (`IDLE/VOICE/RAW/DATA/FACCH/FREE/BUSY/UNDEF`).
   - `"sync_acquired": true` when `idx == 0` and ftype is the SS1
     pattern.

The M-field steers what the rest of the frame contains.

## PSC TCH/SACCH path (S2 / S6 frames)

For PSC frames, `proc_psc_branch` runs the full pipeline:

1. Decode RICH (above). On failure, mark PSC idle and bail.
2. **Dewhiten** the 35-byte PSC payload (32 TCH/FACCH + 3 SACCH/RCH)
   with `WP_PSC_TCH` ([primitives.rs:147-158](../../src/primitives.rs#L147-L158)).
3. Emit `"sacch_slot"` (per-frame nibble + 16-bit data).
4. Run [`proc_psc_sacch`](../../src/decoder/header.rs#L62-L113):
   gather SACCH/RCH bytes into `state.sacch_buf[18]`. The super-frame
   counter (`state.sacch_count`) increments each frame; when 2/10/18
   are reached, the slot pairs are repacked into `state.rch[5]` and
   `state.sacch[2][20]` for the canonical-position emitters above.
5. Emit `"tch"` — raw TCH/FACCH bytes as hex.
6. **Pair-decode** with the previous frame:
   - If `psc.m == Data | Facch`, run [`acch::proc_layer2`](../../src/decoder/acch.rs#L14-L75)
     on `(prev_tch, raw)`.
   - If both `m_eff` and `psc.m` are VOICE, run
     [`voice::proc_voice`](../../src/decoder/voice.rs#L62-L150).
7. Cache `tch + m` in `psc` for the *next* iteration if M-field is
   VOICE/DATA/FACCH, otherwise reset to IDLE.

## SS1 (PICH) path

`proc_ss1_branch` decodes RICH then runs
[`proc_pich`](../../src/decoder/header.rs#L115-L141): dewhiten via
`WP_PCC_TCH` (13 bytes), deinterleave (13×8), slice, K=6 deconvolve
with CRC-6, and emit a `"pich"` sub-object with paging fields:

| Field | Bits | Meaning |
|---|---|---|
| `flag` | 1 | leading flag bit |
| `group` | 1 | group flag |
| `a` | 3 | sub-field a |
| `b` | 3 | sub-field b |
| `c` | 3 | sub-field c |
| `slot` | 5 | slot number |
| `firedep` | 12 | fire department id |
| `station` | 12 | station id |
| `flag2` | 1 | trailing flag |

`psc` is reset to IDLE after a successful SS1 frame.

## Layer-2 multi-frame reassembly

ACCH and L2 data are spread across multiple consecutive frames. The
re-assembly state lives in [`DecoderState`](../../src/state.rs)
(`l2blocks[12*64]`, `l2block_count`, `l2block_last_len`).

`proc_layer2` ([acch.rs:14-75](../../src/decoder/acch.rs#L14-L75)):

1. Combine the two cached frames `f1` (previous) and `f2` (current)
   bit-by-bit: `(f1[i] & 0xaa) | (f2[i] & 0x55)`. This is the ARIB
   pair-merge for L2.
2. Deinterleave (32×8), slice, K=6 R=1/2 deconvolve with CRC-16.
3. The first two output bits encode `(first, last)` flags for the L2
   block.
4. `len_field = l2[0] & 0x3f` is the body length; `body = l2[1..]` is
   the 12-byte payload chunk.
5. Cases:
   - `(first==1, last==1)` — single-frame block; pass to
     [`emit_acch_fields`](../../src/decoder/acch.rs#L82-L155).
   - `first==1, last==0` — start a multi-frame block, allocate
     `len_field + 1` slots.
   - `last==1` — terminal frame, dispatch to
     [`l2_block::proc_l2block`](../../src/decoder/l2_block.rs#L11-L23)
     keyed off the M-field.

The dispatcher in `proc_l2block_inner` handles three kinds of L2
data:

- **FACCH** (M-field == FACCH): a short single-frame command;
  parsed by `proc_l2block_facch`.
- **DATA** (M-field == DATA, len ≥ 40): a fully-formed L2 data block.
  After the common header
  ([l2_block.rs:96-107](../../src/decoder/l2_block.rs#L96-L107))
  containing `text00 from to text01 info_len time message_id`, the
  routine selects:

  - **Text-format** (`l2blocks[39]` is ASCII digit `0..9`):
    [`l2_text::proc_l2block_data_textinfo`](../../src/decoder/l2_text.rs)
    decodes a per-`infotype` layout; over a dozen are recognised
    (0x01, 0x02, 0x10, 0x11, 0x25, 0x2f, 0x31, 0x32, 0x33, 0x38,
    0x3d, 0x3f, 0x40, 0x60, 0x69, 0x6d, 0xa0, …) plus a fallback
    `text07` rest-text emitter.
  - **SENDAI binary** (`len ∈ {63, 79, 84, 121, 312}` and
    `l2blocks[len-3] == 3`):
    [`l2_binary::proc_l2block_sendai`](../../src/decoder/l2_binary.rs#L13-L89)
    handles the regional Sendai-area extension.
  - **Generic binary** (everything else):
    [`l2_binary::proc_l2block_binary`](../../src/decoder/l2_binary.rs#L91-L257)
    dispatches by `info_type` byte: 0x00, 0x01, 0x04, 0x08, 0x0a,
    0x0e, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x20, 0x25, 0x3c,
    0x80, 0x83, 0xf0.

- Anything else falls through (no L2 emission).

## ACCH inner format

`emit_acch_fields` ([acch.rs:82-155](../../src/decoder/acch.rs#L82-L155))
parses an ACCH command frame from a length-prefixed buffer:

- `len` (6 bits, masked from the first byte).
- If `len ≤ 4`, dump as raw `hex`.
- Inspect head byte:
  - `0x01` / `0x02` — short header, 1-byte command + 1 hex byte.
  - `0x04` — `signal` family. Decodes `command`, `subcommand`,
    `subcommand_str` (`off` / `on`), and on subcommand ≠ 8 also
    `type` and `type_str` (`stop` / `notify` / `fire` /
    `ambulance` / `rescue` / `otherwise` / `PA cooperation` /
    `disaster response`).
  - else — generic 1-byte command + 2 hex.
- Then a length-prefixed body: `len1` + `hex1` + `len2` + `hex2`,
  with any trailing bytes treated as Shift_JIS text.

The same parser is reused for SACCH (6-byte body via
`emit_sacch_block_fields`). See
[acch.rs:259-304](../../src/decoder/acch.rs#L259-L304).

## CELP voice path

[`voice::proc_voice`](../../src/decoder/voice.rs#L62-L150) handles
the most complex branch:

1. **Blank-frame check** on `f1[..16]` and `f2[16..]`. If either is
   all-`0x00` or all-`0xff`, emit `"voice":{"error":"blank"}`.
2. Concatenate `f1[..16] || f2[16..]` into a 32-byte CELP frame.
3. **Deinterleave** via `VOICE_INTERLEAVE_MATRIX[256]` (each entry is
   the source bit-position).
4. Repack into 101 dibits and apply the
   `VOICE_CONV_MATRIX[VOICE_MAGIC_TABLE[i]][dibit]` mapping. This
   pre-conditions the input for the K=9 deconvolver.
5. **K=9 R=1/2 deconvolution**
   ([`convo::deconvo29`](../../src/convo.rs#L152-L157), no CRC).
6. **Tail XOR**: `v_tmp[25..32] ^= VOICE_TAIL_MASK`. Then pull 54 more
   bits from `v_tmp[202..256]` directly into `deconvo_result[101..155]`
   (Type-2 voice bits, not convolved).
7. **Hash check** with the embedded 9-bit hash
   (`VOICE_HASH_MAGIC = 0x327`). On mismatch, set
   `deconvo_result[155] = 1` and emit `"hash_ok": false`.
8. **MCA reordering**: reverse-copy `[5..=88]` then `[101..=154]`
   into `celp_mca[0..138]`, append the hash flag at index 138.
9. **CELP_CONV_TABLE permutation**: rebuild a 139-bit `celp_raw`
   array.
10. **Normalisation**: XOR with `CELP_NORMALIZE[18]`.
11. Emit a 36-character lowercase hex string in `voice.celp` (17
    full bytes + final two nibbles).

The output is exactly the bit-stream the embedded CELP synthesizer
expects.

## CELP-only mode

When invoked with `-c` / `--celp`, the decoder emits *only* the 36-hex
CELP payload, one per voice frame, plus a trailing `\n`:

```sh
t61_fd_decoder -c < frames.t61 > voice.celp
```

Frames that did not produce a CELP payload (no_signal, header
errors, blank/failed voice, non-VOICE M-field) emit nothing — output
is suitable for piping straight into a CELP synthesizer without any
post-processing. Implementation:
[mod.rs:128-136](../../src/decoder/mod.rs#L128-L136).

## Convolutional decoders

Both decoders share a common pattern: iterative-deepening backtracking
with in-place flips of the input buffer and `(i, state)` resume so
that recursion picks up from the mismatch point instead of starting
over.

| Decoder | Constraint length | Polynomials | CRC | Max errors |
|---|---|---|---|---|
| `deconvo26` | K=6 R=1/2 | `CONVO_TABLE_26` | none / CRC-6 / CRC-16 (selectable via `t_crc`) | 5 |
| `deconvo29` | K=9 R=1/2 | G1=0x11d, G2=0x1af | none | 8 |

K=6 builds a 32×4 step table once via `OnceLock` so the inner loop is
a single byte fetch per symbol; sentinel `0xff` marks invalid
transitions. K=9 uses `CONVO_TABLE_29[256]` directly.

## CRCs

CRC-6 polynomial `1 + X + X^6`, CRC-16 polynomial
`1 + X^5 + X^12 + X^16` ([primitives.rs:80-103](../../src/primitives.rs#L80-L103)).
Both operate on unpacked-bit input streams (one bit per input byte),
matching the post-deconvolve layout.

## TKY → WGS84 GPS conversion

Coordinates received over the air are in the legacy Tokyo (TKY)
datum. They are decoded from BCD or 24-bit raw, scaled to
*degrees × 1e6*, then converted to WGS84 with a small linear
approximation:

```
wgs84_lat = la − ⌊la / 9 350⌋ + ⌊lo / 57 261⌋ + 4 602
wgs84_lon = lo − ⌊la / 21 721⌋ − ⌊lo / 12 042⌋ + 10 040
```

(see [gps.rs:53-65](../../src/gps.rs#L53-L65)). Either input being 0
short-circuits to 0 (sentinel for "no fix").

The 9-byte BCD form (`decode_degree_bcd`) supports two skip-after-N
flags so the same routine handles separator-stripped (`DDDMMSSSSS`)
and separator-bearing (`DDD-MM-SSSSS`) variants used by SENDAI L2
frames.

## Decoder state recap

| Field | Resets on | Purpose |
|---|---|---|
| `state.sacch_count` | S2 frame, no_signal, SS1, unknown | super-frame counter (0..18) |
| `state.sacch_buf` | implicitly via overwriting | per-slot SACCH/RCH bytes |
| `state.rch` / `state.sacch` | overwriting | repacked at counts 2/10/18 |
| `state.l2blocks` / counts | new alloc, end of block | L2 multi-frame buffer |
| `state.sacch_blocks` / counts | new alloc, end of block | SACCH multi-frame buffer |
| `psc.m` / `psc.tch` | every PSC frame | previous-frame TCH and M-field for pair-decoding |
