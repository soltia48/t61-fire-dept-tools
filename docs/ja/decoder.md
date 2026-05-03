# フレームデコーダ (`t61_fd_decoder`)

デコーダは 48 バイト FDMA フレームを取り、フレーム毎に 1 レコードを
出力します。出力形式は JSONL (既定) または CELP のみの 16 進列
(`-c` / `--celp`) です。内部ではすべてのフレームに対して ARIB プロト
コルスタックを完全に走らせます (ホワイトニング解除、デインタリーブ、
畳み込み復号、CRC 検査、ヘッダ/PSC/PICH/レイヤ 2/音声/SACCH/RCH/GPS の
パーサ)。

## フレーム振り分け

[`Decoder::process_frame`](../../src/decoder/mod.rs#L83-L123) は
バイト `frame[0x17]` と 4 種類の同期ワード候補を見て分岐を決めます。

| 条件 | 分岐 | 備考 |
|---|---|---|
| `frame[0x17] == 0` | `no_signal` | スライサが詰めたプレースホルダ。PSC とアイドル状態をリセット。 |
| `compare_sync_byte(... S6)` | PSC (`S6`) | スーパーフレーム途中。 |
| `compare_sync_byte(... S2)` | PSC (`S2`) | スーパーフレーム先頭、`sacch_count` をリセット。 |
| `compare_sync_byte(... SS1)` | SS1 | センター→端末同期、PICH ページング。 |
| その他 | `unknown_sync` | PSC とアイドル状態をリセット。 |

ここでの同期一致は最大 **3** シンボルエラー許容
(`MAX_SW_ERR = 3`)。スライサ側 (±1) より緩いのは、すでにスライサが
正規パターンに上書き済みだからです。

分岐後は
[`finalize_super_frame`](../../src/decoder/mod.rs#L140-L151) が、
スーパーフレーム位置 2 で RCH、位置 10 で SACCH[0]、位置 18 で
SACCH[1] を出力し、最後に `sacch_count` をリセットします。

## ヘッダ経路

`proc_t61_frame` ([header.rs:11-48](../../src/decoder/header.rs#L11-L48))
はフレームの `0x10..0x17` 区間に対して動作します。

1. **デインタリーブ**: 7 バイトを 8×7 ビットグリッドとして解く
   ([`primitives::interleave`](../../src/primitives.rs#L107-L118))。
2. **スライス**: 各バイトを 4 つの 2 ビットシンボルに分解 (`slice2`)。
3. **畳み込み復号**: K=6 R=1/2
   ([`convo::deconvo26`](../../src/convo.rs#L100-L105)、CRC-6 付)。
4. 失敗時 → `"error":"rich_deconvo"` を出力。
5. 成功時:

   - `"rich"` — 6 つの 2〜3 ビット RICH フィールドを整数にパック直したもの。
   - `"mfield"` — `r[5..7]` の 3 ビットインデックスから取った文字列名
     (`IDLE/VOICE/RAW/DATA/FACCH/FREE/BUSY/UNDEF`)。
   - `"sync_acquired": true` — `idx == 0` かつ ftype が SS1 パターンの場合。

M フィールドが、フレームの残りを何として扱うかを決めます。

## PSC TCH/SACCH 経路 (S2 / S6 フレーム)

PSC フレームでは `proc_psc_branch` がフルパイプラインを実行します。

1. RICH 復号 (上記)。失敗時は PSC をアイドルにして抜ける。
2. **ホワイトニング解除**: 35 バイトの PSC ペイロード (32 バイト
   TCH/FACCH + 3 バイト SACCH/RCH) に対して `WP_PSC_TCH` を XOR
   ([primitives.rs:147-158](../../src/primitives.rs#L147-L158))。
3. `"sacch_slot"` を出力 (フレーム毎のニブル + 16 ビットデータ)。
4. [`proc_psc_sacch`](../../src/decoder/header.rs#L62-L113) を実行:
   SACCH/RCH バイトを `state.sacch_buf[18]` に蓄積。スーパーフレーム
   カウンタ (`state.sacch_count`) が毎フレーム増加し、2/10/18 の各
   位置に達したらスロットペアを `state.rch[5]` と
   `state.sacch[2][20]` に詰め直し、上述の正規位置出力器が消費する。
5. `"tch"` を出力 — 生 TCH/FACCH 32 バイトの 16 進文字列。
6. **前フレームとペア復号**:
   - `psc.m == Data | Facch` のとき、
     [`acch::proc_layer2`](../../src/decoder/acch.rs#L14-L75) を
     `(prev_tch, raw)` で実行。
   - `m_eff` も `psc.m` も VOICE のとき、
     [`voice::proc_voice`](../../src/decoder/voice.rs#L62-L150) を実行。
7. M フィールドが VOICE/DATA/FACCH なら *次回用* に
   `tch + m` を `psc` にキャッシュ。それ以外は IDLE にリセット。

## SS1 (PICH) 経路

`proc_ss1_branch` は RICH 復号後に
[`proc_pich`](../../src/decoder/header.rs#L115-L141) を呼びます。
`WP_PCC_TCH` で 13 バイトをホワイトニング解除し、13×8 でデインタ
リーブ、スライス、K=6 畳み込み復号 (CRC-6) を行ったのち、ページング
情報を `"pich"` サブオブジェクトとして出力します。

| フィールド | ビット | 内容 |
|---|---|---|
| `flag` | 1 | 先頭フラグ |
| `group` | 1 | グループフラグ |
| `a` | 3 | サブフィールド a |
| `b` | 3 | サブフィールド b |
| `c` | 3 | サブフィールド c |
| `slot` | 5 | スロット番号 |
| `firedep` | 12 | 消防本部 ID |
| `station` | 12 | 消防署 ID |
| `flag2` | 1 | 末尾フラグ |

SS1 フレーム成功後 `psc` は IDLE にリセットされます。

## レイヤ 2 マルチフレーム再構築

ACCH と L2 データは複数の連続フレームにまたがります。再構築用の状態
は [`DecoderState`](../../src/state.rs) (`l2blocks[12*64]`、
`l2block_count`、`l2block_last_len`) が保持します。

`proc_layer2` ([acch.rs:14-75](../../src/decoder/acch.rs#L14-L75)):

1. キャッシュした 2 フレーム `f1` (前) と `f2` (現) をビット単位で
   合成: `(f1[i] & 0xaa) | (f2[i] & 0x55)`。これが ARIB の L2 ペア
   合成。
2. デインタリーブ (32×8)、スライス、K=6 R=1/2 畳み込み復号
   (CRC-16)。
3. 出力先頭 2 ビットが L2 ブロックの `(first, last)` フラグ。
4. `len_field = l2[0] & 0x3f` が本体長、`body = l2[1..]` が 12 バイト
   ペイロードチャンク。
5. ケース:
   - `(first==1, last==1)` — 単一フレームブロック。
     [`emit_acch_fields`](../../src/decoder/acch.rs#L82-L155) に渡す。
   - `first==1, last==0` — マルチフレームブロックの開始。
     `len_field + 1` スロットを確保。
   - `last==1` — 終端フレーム。M フィールドに応じて
     [`l2_block::proc_l2block`](../../src/decoder/l2_block.rs#L11-L23) に分岐。

`proc_l2block_inner` のディスパッチャは 3 種類の L2 データを扱います。

- **FACCH** (M フィールド == FACCH): 単一フレームの短いコマンド。
  `proc_l2block_facch` で解析。
- **DATA** (M フィールド == DATA、`len ≥ 40`): 完全な L2 データブロック。
  共通ヘッダ (
  [l2_block.rs:96-107](../../src/decoder/l2_block.rs#L96-L107))
  `text00 from to text01 info_len time message_id` の後で次に分岐:

  - **テキスト形式** (`l2blocks[39]` が ASCII 数字 `0..9`):
    [`l2_text::proc_l2block_data_textinfo`](../../src/decoder/l2_text.rs)
    が `infotype` 別レイアウトで復号。10 種類以上の infotype に対応
    (0x01, 0x02, 0x10, 0x11, 0x25, 0x2f, 0x31, 0x32, 0x33, 0x38,
    0x3d, 0x3f, 0x40, 0x60, 0x69, 0x6d, 0xa0、…) と、未対応の場合の
    `text07` 残テキスト出力を持つ。
  - **仙台バイナリ** (`len ∈ {63, 79, 84, 121, 312}` かつ
    `l2blocks[len-3] == 3`):
    [`l2_binary::proc_l2block_sendai`](../../src/decoder/l2_binary.rs#L13-L89)
    が仙台地区拡張形式を処理。
  - **汎用バイナリ** (上記以外):
    [`l2_binary::proc_l2block_binary`](../../src/decoder/l2_binary.rs#L91-L257)
    が `info_type` バイトで分岐 (0x00, 0x01, 0x04, 0x08, 0x0a,
    0x0e, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x20, 0x25, 0x3c,
    0x80, 0x83, 0xf0)。

- それ以外は素通し (L2 出力なし)。

## ACCH 内側形式

`emit_acch_fields` ([acch.rs:82-155](../../src/decoder/acch.rs#L82-L155))
は、長さ前置型のバッファから ACCH コマンドフレームを解析します。

- `len` (先頭バイトの下位 6 ビット)。
- `len ≤ 4` なら本体を生 `hex` として出力して終了。
- 先頭バイトを覗き見:
  - `0x01` / `0x02` — 短ヘッダ、1 バイトコマンド + 1 hex バイト。
  - `0x04` — `signal` 系列。`command`、`subcommand`、
    `subcommand_str` (`off` / `on`) を出力。`subcommand != 8` の
    場合は `type` と `type_str` (`stop` / `notify` / `fire` /
    `ambulance` / `rescue` / `otherwise` / `PA cooperation` /
    `disaster response`) を出力。
  - その他 — 汎用 1 バイトコマンド + 2 hex。
- 続いて長さ前置の本体 `len1` + `hex1` + `len2` + `hex2`、末尾は
  Shift_JIS テキストとして扱う。

このパーサは SACCH (6 バイト本体) でも再利用されます
(`emit_sacch_block_fields`、
[acch.rs:259-304](../../src/decoder/acch.rs#L259-L304))。

## CELP 音声経路

[`voice::proc_voice`](../../src/decoder/voice.rs#L62-L150) は最も複雑
な分岐です。

1. **空フレーム検査**: `f1[..16]` と `f2[16..]` のいずれかが全 `0x00`
   または全 `0xff` なら `"voice":{"error":"blank"}` を出力。
2. `f1[..16] || f2[16..]` を連結して 32 バイトの CELP フレームを構築。
3. **デインタリーブ**: `VOICE_INTERLEAVE_MATRIX[256]` (各エントリは
   元ビット位置)。
4. 101 個の dibit に詰め直し、
   `VOICE_CONV_MATRIX[VOICE_MAGIC_TABLE[i]][dibit]` 写像を適用。これが
   K=9 復号器の入力前処理。
5. **K=9 R=1/2 畳み込み復号**
   ([`convo::deconvo29`](../../src/convo.rs#L152-L157)、CRC なし)。
6. **テール XOR**: `v_tmp[25..32] ^= VOICE_TAIL_MASK`。続いて
   `v_tmp[202..256]` から 54 ビットを直接 `deconvo_result[101..155]`
   へ取り出す (Type-2 音声ビット、畳み込みなし)。
7. **ハッシュ検査**: 9 ビット埋込みハッシュ
   (`VOICE_HASH_MAGIC = 0x327`)。不一致なら
   `deconvo_result[155] = 1` をセットし `"hash_ok": false` を出力。
8. **MCA 並べ替え**: `[5..=88]` と `[101..=154]` を逆順コピーして
   `celp_mca[0..138]` を構築、index 138 にハッシュフラグを付加。
9. **CELP_CONV_TABLE 並べ替え**: 139 ビットの `celp_raw` を再構築。
10. **正規化**: `CELP_NORMALIZE[18]` で XOR。
11. 36 文字の小文字 16 進文字列を `voice.celp` に出力 (17 バイト
    フル + 末尾 2 ニブル)。

これがそのまま CELP シンセサイザの入力ビットストリームになります。

## CELP のみモード

`-c` / `--celp` 指定時は、音声フレーム毎に 36 文字 16 進の CELP ペイ
ロードのみを 1 行ずつ出力します。

```sh
t61_fd_decoder -c < frames.t61 > voice.celp
```

CELP ペイロードを生成しなかったフレーム (no_signal、ヘッダエラー、
空音声、復号失敗、非 VOICE M フィールド) は何も出力しません。CELP
シンセサイザへ後処理なしでパイプ可能です。実装は
[mod.rs:128-136](../../src/decoder/mod.rs#L128-L136)。

## 畳み込み復号器

両復号器は共通の戦略 — 入力バッファをその場で反転しつつ
`(i, state)` で再開する反復深化バックトラッキング — を使います。
これにより、再帰がオフセット 0 まで巻き戻すのではなく、不一致点から
再開できます。

| 復号器 | 拘束長 | 多項式 | CRC | 最大誤り |
|---|---|---|---|---|
| `deconvo26` | K=6 R=1/2 | `CONVO_TABLE_26` | なし / CRC-6 / CRC-16 (`t_crc` で選択) | 5 |
| `deconvo29` | K=9 R=1/2 | G1=0x11d, G2=0x1af | なし | 8 |

K=6 は `OnceLock` で 32×4 のステップ表を一度だけ構築するので内側
ループは 1 シンボルあたり 1 バイトの取り出しで済みます。`0xff` が
無効遷移のセンチネル。K=9 は `CONVO_TABLE_29[256]` を直接参照します。

## CRC

CRC-6 多項式 `1 + X + X^6`、CRC-16 多項式
`1 + X^5 + X^12 + X^16` (
[primitives.rs:80-103](../../src/primitives.rs#L80-L103))。いずれも
1 バイトに 1 ビットの形式の入力に対して計算され、復号後のレイアウト
と一致します。

## TKY → WGS84 GPS 変換

電波で受信される座標は旧日本測地系 (TKY) です。BCD あるいは 24 ビット
生から *度 × 1e6* にデコードしたあと、線形近似で WGS84 に変換します。

```
wgs84_lat = la − ⌊la / 9 350⌋ + ⌊lo / 57 261⌋ + 4 602
wgs84_lon = lo − ⌊la / 21 721⌋ − ⌊lo / 12 042⌋ + 10 040
```

(参照 [gps.rs:53-65](../../src/gps.rs#L53-L65))。入力どちらかが 0 なら
0 を返します ("無測位" のセンチネル)。

9 バイトの BCD 形式 (`decode_degree_bcd`) は 2 つの skip-after-N フラ
グを取り、区切り文字なし (`DDDMMSSSSS`) と区切り文字あり
(`DDD-MM-SSSSS`) の両形式を同じルーチンで扱えます。後者は仙台 L2
フレームで使われます。

## デコーダ状態の早見表

| フィールド | リセットタイミング | 用途 |
|---|---|---|
| `state.sacch_count` | S2 フレーム / no_signal / SS1 / unknown | スーパーフレームカウンタ (0..18) |
| `state.sacch_buf` | 上書き | スロット毎 SACCH/RCH バイト |
| `state.rch` / `state.sacch` | 上書き | カウント 2/10/18 で詰め直し |
| `state.l2blocks` ほか | 新規 alloc / ブロック終端 | L2 マルチフレームバッファ |
| `state.sacch_blocks` ほか | 新規 alloc / ブロック終端 | SACCH マルチフレームバッファ |
| `psc.m` / `psc.tch` | PSC フレーム毎 | ペア復号用、前フレームの TCH と M フィールド |
