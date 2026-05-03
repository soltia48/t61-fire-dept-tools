# JSONL / CELP output reference

## JSONL records

`t61_fd_decoder` (default mode) emits exactly one JSON object per
input frame, one line each. Keys preserve insertion order via
`serde_json`'s `preserve_order` feature, so each record's keys appear
in the order the decoder filled them in.

### Top-level fields

Two fields are always present:

| Key | Type | Description |
|---|---|---|
| `frame` | integer | zero-based frame counter |
| `timestamp` | string | RFC 3339, millisecond precision, local timezone, recorded when decoding ran (not when transmitted) |

Then exactly one of:

| Key | When |
|---|---|
| `type: "no_signal"` | Slicer placeholder; no data this frame. |
| `type: "unknown_sync"` | Frame had data but no sync match. |
| `sync: "S2" \| "S6" \| "SS1"` | Real frame; matches the ARIB sync word. |

### S2 / S6 frame fields

Frames matching the **PSC** branch carry the following:

| Key | Type | Source | Notes |
|---|---|---|---|
| `rich` | array of integers (length 6) | header | RICH payload, packed back into 2-3 bit fields |
| `mfield` | string | header | one of `IDLE/VOICE/RAW/DATA/FACCH/FREE/BUSY/UNDEF` |
| `error` | string | header | only on RICH deconvo failure: `"rich_deconvo"` |
| `sacch_slot` | object | dewhitened payload | per-frame SACCH slot: `{rch_nib, data16}` |
| `tch` | hex string | dewhitened payload | 32-byte raw TCH/FACCH |
| `voice` | object | M-field == VOICE | see CELP section below |
| `layer2` | object | M-field == DATA / FACCH | see Layer-2 section |
| `rch` | object | super-frame position 2 | `{raw, bits}` |
| `sacch` | object | super-frame positions 10 / 18 | see SACCH section |

### SS1 frame fields

| Key | Type | Notes |
|---|---|---|
| `rich` | array | as above |
| `mfield` | string | as above |
| `sync_acquired` | bool | `true` when `idx == 0` |
| `pich` | object | paging — see below |

### `voice`

```jsonc
"voice": {
    "celp": "0123456789abcdef0123456789abcdef0123",   // 36 hex chars
    "hash_ok": true                                    // false on hash mismatch
}
```

When the frame is blank or deconvolution fails:

```json
"voice": {"error": "blank"}
"voice": {"error": "deconvo_failed"}
```

The 36-character CELP hex is exactly the bitstream a CELP synthesizer
expects (17 full bytes, then 4 bits of the 18th byte, then the
embedded hash flag).

### `pich`

```jsonc
"pich": {
    "flag":     0,        // 1 bit
    "group":    0,        // 1 bit
    "a":        0,        // 3 bits
    "b":        0,        // 3 bits
    "c":        0,        // 3 bits
    "slot":     0,        // 5 bits
    "firedep":  0,        // 12 bits — fire department id
    "station":  0,        // 12 bits — station id
    "flag2":    0         // 1 bit
}
```

On deconvo failure: `"error": "pich_deconvo"`.

### `layer2`

The base shape of every L2 frame:

```jsonc
"layer2": {
    "first":     true,
    "last":      true,
    "len_field": 12,
    "body":      "...",       // 12-byte hex
    "acch":      { ... },     // when first && last (single-frame)
    "facch":     { ... },     // when M=FACCH and last==1 of multi-frame
    "data":      { ... }      // when M=DATA and last==1 of multi-frame
}
```

#### `layer2.acch`

For single-frame L2 blocks (first == last == 1):

| Key | Notes |
|---|---|
| `len` | declared length |
| `hex` | (only when `len ≤ 4`) raw payload |
| `command` | command byte |
| `command_str` | only when `command == 0x04` (`"signal"`) |
| `subcommand` | only when `command == 0x04` |
| `subcommand_str` | `"off"` / `"on"` |
| `type` | only when `command == 0x04 && subcommand != 8` |
| `type_str` | `"stop"`, `"notify"`, `"fire"`, `"ambulance"`, `"rescue"`, `"otherwise"`, `"PA cooperation"`, `"disaster response"` |
| `hex0` | small leading hex chunk |
| `len1` | first sub-block length |
| `hex1` | first sub-block bytes |
| `len2` | second sub-block length |
| `hex2` | second sub-block bytes |
| `text` | trailing text (Shift_JIS decoded, NUL bytes stripped) |

#### `layer2.facch`

```jsonc
"facch": {
    "len":  ...,                   // total bytes
    "raw":  "...",                 // hex of full body
    "hex00": "...", "len01": "...", "hex01": "...",
    "len02": "...", "hex02": "...",
    "text00": "..."
}
```

#### `layer2.data`

Common header fields (always present once `len ≥ 42`):

| Key | Bytes | Meaning |
|---|---|---|
| `text00` | 1 | first byte of body |
| `from` | 4 | source unit |
| `to` | 4 | destination unit |
| `text01` | 7 | header text |
| `info_len` | 3 | info length, ASCII decimal |
| `time` | 2 | timestamp |
| `message_id` | 4 | message id |

Then either text-format or binary-format extensions. The branch is
chosen by `l2blocks[39]`:

- ASCII digit (`0x30..=0x39`): `"info_type_kind": "text"` — the
  text-info branch fires.
- Otherwise: `"info_type_byte": <int>` — the binary or SENDAI branch
  fires.

##### Text-format infotypes

`infotype` is read from `l2blocks[40..42]` as two ASCII hex digits.
Recognised infotypes (each emits its own field set):

`0x01`, `0x02`, `0x10`, `0x11`, `0x25`, `0x2f`, `0x31`, `0x32`,
`0x33`, `0x38`, `0x3d`, `0x3f`, `0x40`, `0x60`, `0x69`, `0x6d`,
`0xa0`. Anything else falls through to a generic `text07` rest-text
emitter.

Common text-format fields include `info_inner_len`, `info_type`,
`vehicle_id`, `text02..text27`, `date1_year`, `time1`, `address`,
`landmark`, `mapinfo`, `direction`, `distance`, `gps`, `speed`,
`action`, `cause`, `subaction`, `name`, `telephone`, `block_count`,
`data_count`, `data`, `message_len`, `message`, `type`, `building`.

GPS appears as `"gps": {"lat": <float>, "lon": <float>}` with
WGS84 degrees. Latitude/longitude of `0` short-circuits to no `gps`
emission (treat as "no fix").

##### SENDAI-format

Triggered when `len ∈ {63, 79, 84, 121, 312}` and
`l2blocks[len-3] == 3`. Layout:

```jsonc
"data": {
    "source_id":      "...",   // 2 hex
    "info_type":      "...",   // 1 hex
    "destination_id": "...",   // 2 hex
    "hex01..hex04":   "...",
    // then a per-info_type variant emitting date/time/text + GPS,
    // ending with rest_hex("hex05")
}
```

`info_type`s recognised: `0x04`, `0x10..0x15`, `0x18`, `0x1b..0x1d`,
`0x21`, `0x23..0x25`, `0x27`, `0x2a..0x2e`, `0xdd`, `0xeb`. The
`0x1b/0x1d` variant carries a full incident package (two GPS slots,
multiple date/time pairs, address/name/mapinfo).

##### Binary-format

Generic binary L2 dispatched on `l2blocks[39]`. Fields per branch
include `destination_id`, `date_year`, `body_time`, `gps`, `nav`
(`speed_kmh`, `dir_idx`, `dir`, `gps_status_code`, `gps_status`),
plus a length-prefixed `data_hex` for `info_type ∈ {0x10, 0x12,
0x13, 0x14, 0x15}`. The `0x80` variant carries a vehicle dispatch
package: `gps`, `action`, `cause`, `address`, `name`, `mapinfo`,
`vehicle_count` (4-digit BCD count) followed by that many
`vehicle_id` hex slots.

### `rch`

Emitted at super-frame position 2. Three "invalid" patterns in the
buffer suppress emission entirely:

```jsonc
"rch": {
    "raw":  "9991199900",   // 5-byte hex
    "bits": [0,1,0,...]     // 8 deconvolved bits
}
```

### `sacch`

Emitted at super-frame positions 10 and 18 with the same three
"invalid" patterns suppressed:

```jsonc
"sacch": {
    "raw":       "...",     // 20-byte hex
    "first":     true,
    "last":      true,
    "len_field": 6,
    "body":      "...",
    // then either acch fields (single-frame) or
    // a "block" sub-object holding multi-frame fields
}
```

The `"block"` variant uses `len`, `hex0`, `len1`, `hex1`, `len2`,
`hex2`, `text` (parallel to ACCH but without the signal-command
parsing).

### Examples

```json
{"frame":0,"timestamp":"2026-04-30T14:21:20.868+09:00","type":"no_signal"}
{"frame":1,"timestamp":"2026-04-30T14:21:20.868+09:00","sync":"S6","error":"rich_deconvo"}
{"frame":42,"timestamp":"2026-04-30T14:21:29.268+09:00","sync":"S2","rich":[1,0,1,3,2,5],"mfield":"VOICE","sacch_slot":{"rch_nib":3,"data16":12345},"tch":"...","voice":{"celp":"...","hash_ok":true}}
```

## CELP-only output

`-c` / `--celp` switches to a much terser format: 36 lowercase hex
characters per voice frame, one per line. No prefixes, no
timestamps, no records for non-voice frames. The output is suitable
for piping straight into a CELP synthesizer (e.g. an MCA-style
toll-quality decoder):

```sh
t61_fd_decoder -c < frames.t61 > voice.celp
```

Frames suppressed (no output):

- `no_signal` placeholders
- `unknown_sync`
- frames with `error` (RICH or PICH deconvolution failure)
- voice frames flagged blank or deconvolution-failed
- frames whose M-field is not VOICE
- non-S2 / non-S6 frames

The 36-character output is the same string that JSONL mode places
under `voice.celp`, so a JSONL log can be filtered to
CELP-stream-equivalent output with:

```sh
jq -r '.voice.celp // empty' frames.jsonl
```
