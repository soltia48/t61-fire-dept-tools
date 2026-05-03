# JSONL / CELP 出力リファレンス

## JSONL レコード

`t61_fd_decoder` は既定モードでフレーム毎に正確に 1 つの JSON オブ
ジェクトを 1 行で出力します。`serde_json` の `preserve_order` 機能で
キーの挿入順序を保持しているため、各レコードのキーはデコーダが書き
込んだ順に並びます。

### トップレベルフィールド

常に存在するフィールドは 2 つ。

| キー | 型 | 説明 |
|---|---|---|
| `frame` | 整数 | 0 起点のフレームカウンタ |
| `timestamp` | 文字列 | RFC 3339、ミリ秒精度、ローカルタイムゾーン。送信時刻ではなくデコード時刻を記録 |

そのうえで次のうち 1 つが必ず付きます。

| キー | タイミング |
|---|---|
| `type: "no_signal"` | スライサのプレースホルダ。このフレームにはデータなし |
| `type: "unknown_sync"` | データはあったが同期一致しなかった |
| `sync: "S2" \| "S6" \| "SS1"` | 実フレーム。ARIB 同期ワードのいずれかに一致 |

### S2 / S6 フレームのフィールド

PSC 分岐に入ったフレームは次を含み得ます。

| キー | 型 | 由来 | 備考 |
|---|---|---|---|
| `rich` | 整数配列 (長さ 6) | header | RICH ペイロードを 2〜3 ビット幅にパック直したもの |
| `mfield` | 文字列 | header | `IDLE/VOICE/RAW/DATA/FACCH/FREE/BUSY/UNDEF` のいずれか |
| `error` | 文字列 | header | RICH 復号失敗時のみ: `"rich_deconvo"` |
| `sacch_slot` | オブジェクト | ホワイトニング解除後 | フレーム毎の SACCH スロット: `{rch_nib, data16}` |
| `tch` | 16 進文字列 | ホワイトニング解除後 | 32 バイト生 TCH/FACCH |
| `voice` | オブジェクト | M フィールド == VOICE | CELP セクション参照 |
| `layer2` | オブジェクト | M フィールド == DATA / FACCH | レイヤ 2 セクション参照 |
| `rch` | オブジェクト | スーパーフレーム位置 2 | `{raw, bits}` |
| `sacch` | オブジェクト | スーパーフレーム位置 10 / 18 | SACCH セクション参照 |

### SS1 フレームのフィールド

| キー | 型 | 備考 |
|---|---|---|
| `rich` | 配列 | 上記と同じ |
| `mfield` | 文字列 | 上記と同じ |
| `sync_acquired` | bool | `idx == 0` のとき `true` |
| `pich` | オブジェクト | ページング (下記) |

### `voice`

```jsonc
"voice": {
    "celp": "0123456789abcdef0123456789abcdef0123",   // 36 文字 16 進
    "hash_ok": true                                    // ハッシュ不一致なら false
}
```

空フレーム / 復号失敗時:

```json
"voice": {"error": "blank"}
"voice": {"error": "deconvo_failed"}
```

36 文字の CELP 16 進は CELP シンセサイザがそのまま受け取れるビット
列 (17 バイトフル + 18 バイト目の 4 ビット + 埋込みハッシュフラグ)
です。

### `pich`

```jsonc
"pich": {
    "flag":     0,        // 1 ビット
    "group":    0,        // 1 ビット
    "a":        0,        // 3 ビット
    "b":        0,        // 3 ビット
    "c":        0,        // 3 ビット
    "slot":     0,        // 5 ビット
    "firedep":  0,        // 12 ビット — 消防本部 ID
    "station":  0,        // 12 ビット — 消防署 ID
    "flag2":    0         // 1 ビット
}
```

復号失敗時は `"error": "pich_deconvo"`。

### `layer2`

各 L2 フレーム共通の基本形:

```jsonc
"layer2": {
    "first":     true,
    "last":      true,
    "len_field": 12,
    "body":      "...",       // 12 バイトの 16 進
    "acch":      { ... },     // first && last の単一フレーム時
    "facch":     { ... },     // M=FACCH のマルチフレーム終端時
    "data":      { ... }      // M=DATA のマルチフレーム終端時
}
```

#### `layer2.acch`

単一フレーム L2 ブロック (first == last == 1) の場合:

| キー | 備考 |
|---|---|
| `len` | 宣言長 |
| `hex` | (`len ≤ 4` のときのみ) 生ペイロード |
| `command` | コマンドバイト |
| `command_str` | `command == 0x04` のとき (`"signal"`) |
| `subcommand` | `command == 0x04` のとき |
| `subcommand_str` | `"off"` / `"on"` |
| `type` | `command == 0x04 && subcommand != 8` のとき |
| `type_str` | `"stop"` / `"notify"` / `"fire"` / `"ambulance"` / `"rescue"` / `"otherwise"` / `"PA cooperation"` / `"disaster response"` |
| `hex0` | 先頭の小さな 16 進塊 |
| `len1` | 第 1 サブブロック長 |
| `hex1` | 第 1 サブブロックバイト列 |
| `len2` | 第 2 サブブロック長 |
| `hex2` | 第 2 サブブロックバイト列 |
| `text` | 末尾テキスト (Shift_JIS 復号、NUL バイト除去) |

#### `layer2.facch`

```jsonc
"facch": {
    "len":  ...,                   // 総バイト数
    "raw":  "...",                 // 本体全体の 16 進
    "hex00": "...", "len01": "...", "hex01": "...",
    "len02": "...", "hex02": "...",
    "text00": "..."
}
```

#### `layer2.data`

共通ヘッダフィールド (`len ≥ 42` で常時出力):

| キー | バイト | 内容 |
|---|---|---|
| `text00` | 1 | 本体先頭バイト |
| `from` | 4 | 送信元ユニット |
| `to` | 4 | 宛先ユニット |
| `text01` | 7 | ヘッダテキスト |
| `info_len` | 3 | info 長 (ASCII 数字) |
| `time` | 2 | 時刻 |
| `message_id` | 4 | メッセージ ID |

そのあとはテキスト形式かバイナリ形式の拡張に分岐します。判定は
`l2blocks[39]` で:

- ASCII 数字 (`0x30..=0x39`): `"info_type_kind": "text"` → text-info
  分岐へ。
- それ以外: `"info_type_byte": <整数>` → バイナリ / 仙台分岐へ。

##### テキスト形式 infotype

`infotype` は `l2blocks[40..42]` から 2 桁の ASCII 16 進として読み
取られます。各 infotype 別にフィールドセットが出力されます。サポート
対象:

`0x01`, `0x02`, `0x10`, `0x11`, `0x25`, `0x2f`, `0x31`, `0x32`,
`0x33`, `0x38`, `0x3d`, `0x3f`, `0x40`, `0x60`, `0x69`, `0x6d`,
`0xa0`。それ以外は `text07` 残テキスト出力にフォールバックします。

代表的なフィールドには `info_inner_len`、`info_type`、`vehicle_id`、
`text02..text27`、`date1_year`、`time1`、`address`、`landmark`、
`mapinfo`、`direction`、`distance`、`gps`、`speed`、`action`、
`cause`、`subaction`、`name`、`telephone`、`block_count`、
`data_count`、`data`、`message_len`、`message`、`type`、`building`
があります。

GPS は `"gps": {"lat": <float>, "lon": <float>}` (WGS84 度) として
現れます。緯度・経度のいずれかが 0 のときは「無測位」として `gps`
の出力自体を抑制します。

##### 仙台形式 (SENDAI)

`len ∈ {63, 79, 84, 121, 312}` かつ `l2blocks[len-3] == 3` のとき
発火します。

```jsonc
"data": {
    "source_id":      "...",   // 2 hex
    "info_type":      "...",   // 1 hex
    "destination_id": "...",   // 2 hex
    "hex01..hex04":   "...",
    // info_type 別の派生で日時/テキスト + GPS を出力し、
    // 末尾に rest_hex("hex05") で残バイトを出す
}
```

サポート対象 `info_type`: `0x04`, `0x10..0x15`, `0x18`, `0x1b..0x1d`,
`0x21`, `0x23..0x25`, `0x27`, `0x2a..0x2e`, `0xdd`, `0xeb`。
`0x1b` / `0x1d` は事案一式 (GPS スロット 2 つ、複数の日時、住所/
氏名/地図情報) を運びます。

##### バイナリ形式

汎用バイナリ L2 は `l2blocks[39]` で分岐します。各分岐のフィールド
には `destination_id`、`date_year`、`body_time`、`gps`、`nav`
(`speed_kmh`、`dir_idx`、`dir`、`gps_status_code`、`gps_status`)
などがあり、`info_type ∈ {0x10, 0x12, 0x13, 0x14, 0x15}` では長さ
前置の `data_hex` を含みます。`0x80` は車両出動一式 (`gps`、
`action`、`cause`、`address`、`name`、`mapinfo`、4 桁 BCD の
`vehicle_count` 後にその数だけ `vehicle_id` 16 進スロット) を運び
ます。

### `rch`

スーパーフレーム位置 2 で出力。3 種類の「無効」パターンに該当する
ときは出力を抑制します。

```jsonc
"rch": {
    "raw":  "9991199900",   // 5 バイト 16 進
    "bits": [0,1,0,...]     // 復号後 8 ビット
}
```

### `sacch`

スーパーフレーム位置 10 と 18 で出力。同じく 3 種類の「無効」パター
ンを抑制します。

```jsonc
"sacch": {
    "raw":       "...",     // 20 バイト 16 進
    "first":     true,
    "last":      true,
    "len_field": 6,
    "body":      "...",
    // 単一フレーム時は acch フィールド、
    // マルチフレーム時は "block" サブオブジェクト
}
```

`"block"` 派生は `len`、`hex0`、`len1`、`hex1`、`len2`、`hex2`、
`text` を持ちます (ACCH と並行ですが signal コマンド解析はなし)。

### 例

```json
{"frame":0,"timestamp":"2026-04-30T14:21:20.868+09:00","type":"no_signal"}
{"frame":1,"timestamp":"2026-04-30T14:21:20.868+09:00","sync":"S6","error":"rich_deconvo"}
{"frame":42,"timestamp":"2026-04-30T14:21:29.268+09:00","sync":"S2","rich":[1,0,1,3,2,5],"mfield":"VOICE","sacch_slot":{"rch_nib":3,"data16":12345},"tch":"...","voice":{"celp":"...","hash_ok":true}}
```

## CELP のみ出力

`-c` / `--celp` を指定すると、音声フレーム毎に 36 文字の小文字 16 進
を 1 行ずつだけ出力する簡素なフォーマットになります。プレフィクス・
タイムスタンプ・非音声フレームのレコードはいずれも出ません。出力は
CELP シンセサイザ (例えば MCA トールクオリティ系のデコーダ) に
そのままパイプ可能です。

```sh
t61_fd_decoder -c < frames.t61 > voice.celp
```

抑制 (出力しない) されるのは以下:

- `no_signal` プレースホルダ
- `unknown_sync`
- `error` 付き (RICH / PICH 復号失敗) フレーム
- 空判定または復号失敗となった音声フレーム
- M フィールドが VOICE 以外
- 非 S2 / 非 S6 フレーム

CELP の 36 文字は JSONL モードの `voice.celp` と同じ文字列なので、
JSONL ログから CELP ストリームを取り出すには:

```sh
jq -r '.voice.celp // empty' frames.jsonl
```
