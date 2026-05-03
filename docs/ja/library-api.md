# ライブラリ API (`t61_fd` crate)

`t61-fire-dept-tools` はスライサとデコーダを Rust ライブラリ クレート
(`t61_fd`) として提供しており、他の Rust プログラムへ組み込めます。
2 つのバイナリは、このライブラリの薄い `main()` ラッパに過ぎません。

## Cargo の指定

```toml
[dependencies]
t61-fire-dept-tools = { path = "..." }   # Cargo.toml のパッケージ名。ライブラリ名は t61_fd
chrono   = { version = "0.4", default-features = false, features = ["clock"] }
encoding_rs = "0.8"
serde_json = { version = "1", features = ["preserve_order"] }
```

ライブラリは edition-2024 を対象としており、Rust 1.85 以降が必要です。

## 公開 API

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

(出典 [`src/lib.rs`](../../src/lib.rs))

## 最小例

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

- 入力は 4 シンボル/バイトのパックド形式 (`arib_t61_rx.py
  --packed-out -` の出力フォーマット)。
- `next_frame` はストリーム終端で `Ok(None)` を返し、それ以降
  イテレータは fused になります。
- 読み出し粒度の注意: 入力ソースが短い読み出しを返すと、フル読み出
  しの場合と異なる (が、いずれも有効な) フレーム決定になります。
  通常ファイル入力で決定的なフレーミングが必要なら、生ファイル記述子
  ラッパを使用してください。

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

内部状態:

- `state: DecoderState` — マルチフレームバッファ (スーパーフレーム
  カウンタ、L2 / SACCH ブロック再構築、RCH / SACCH スロット)。
- `psc: PscState` — 前フレームの TCH と M フィールド。PSC 分岐で
  連続フレームをペア復号するために使用。

`process_frame` を呼ぶたびに、挿入順を保つ `serde_json::Map<String,
Value>` が構築され、適切なプロトコル分岐が走り、`OutputMode::Json`
なら JSONL を 1 行、`OutputMode::CelpOnly` なら `voice.celp` がある
ときに限り 36 文字 16 進を 1 行出力します。

## 状態型

### `MField`

```rust
pub enum MField {
    Idle = 0, Voice = 1, Raw = 2, Data = 3,
    Facch = 4, Free = 5, Busy = 6, Undef = 7,
}

impl MField {
    pub fn from_idx(idx: u8) -> Self;   // 3 ビット RICH インデックス
    pub fn name(self) -> &'static str;  // JSONL 出力で使う文字列
}
```

### `PscState`

```rust
#[derive(Default)]
pub struct PscState {
    pub m: MField,        // 前フレームの M フィールド
    pub tch: [u8; 32],    // 前フレームの TCH/FACCH バイト列
}
```

`no_signal` / SS1 / `unknown_sync` フレームで既定値にリセット。
PSC フレームでは前フレームのデータを保持して `(prev, cur)` のペア
合成 (L2・音声経路) を可能にします。

### `DecoderState`

```rust
pub struct DecoderState {
    pub sacch_count:        usize,        // スーパーフレームカウンタ (0..18)
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

メソッド:

| メソッド | 用途 |
|---|---|
| `new()` / `Default::default()` | ゼロ初期化 |
| `init_l2block` / `init_sacch_block` | ブロックバッファをリセット |
| `alloc_l2block(count)` / `alloc_sacch_block(count)` | `count` スロットの新規マルチフレームブロック開始 |
| `assemble_l2block(f_pos, src)` / `assemble_sacch_block(f_pos, src)` | 1 スロット書き込み |
| `reset_idle()` | 完全アイドルリセット (no_signal / SS1 / unknown sync) |
| `l2block_total_len()` / `sacch_block_total_len()` | 連結後の長さ |

## サブモジュール一覧

- [`primitives`](../../src/primitives.rs) — `bit_test`、`bit_set`、
  `pack_bits_be`、`parse_2digit`、`parse_3digit`、`Cursor`、`crc6`、
  `crc16`、`interleave`、`slice2`、`compare_sync_byte`、
  `dewhite_psc_tch`、`dewhite_pich`。
- [`convo`](../../src/convo.rs) — `deconvo26`、`deconvo29`、
  `DecodeError`。
- [`gps`](../../src/gps.rs) — `decode_latitude_24`、
  `decode_longitude_24`、`decode_degree_bcd`、`tky_to_wgs84_lat`、
  `tky_to_wgs84_lon`、`gps_status_name`、
  `acch_signal_subcommand_name`、`acch_signal_type_name`。
- [`json`](../../src/json.rs) — `text_value` (Shift_JIS 復号 + NUL
  バイト除去の JSON 文字列)、`hex_value`、`FieldEmitter`。
- [`slicer`](../../src/slicer.rs) — `Slicer`、`FRAME_BYTES`。
- [`state`](../../src/state.rs) — `DecoderState`、`PscState`、
  `MField`。
- [`tables`](../../src/tables.rs) — 同期ワード、ホワイトニングパター
  ン、畳み込み表、音声インタリーブ / 変換 / 正規化表、方位文字列。

## カスタムソースからの構築

`std::io::Read` を実装し 4 シンボル/バイトのパックドフォーマットを
出すものなら何でもスライサに流せます。`std::io::Write` を実装する
ものなら何でもデコーダの出力先になります。リアルタイムパイプライン
向けには生ファイル記述子をラップしてください。

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

`t61_frame_slicer` と `t61_fd_decoder` バイナリが行っているのは厳密
にこれです。Rust 側のバッファリングなし、フレーム毎の出力が即時に
フラッシュされます。
