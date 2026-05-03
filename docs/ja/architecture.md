# アーキテクチャ概観

`t61-fire-dept-tools` は、消防無線で利用される **ARIB STD-T61 v1.2 part
2** の SCPC/FDMA 下り回線を復調・復号するツール群です。リポジトリは
3 段の Unix パイプラインで構成されており、各段は独立して使うことも、
リアルタイムにエンドツーエンドで連結することもできます。

```
┌──────────────┐  cf32   ┌──────────────────┐ 4 sym/byte ┌─────────────────┐ 48 B/frame ┌──────────────────┐ JSONL / CELP
│ SDR (HackRF, │────────▶│ arib_t61_rx.py   │───────────▶│ t61_frame_slicer│───────────▶│ t61_fd_decoder   │──────────────▶
│ RTL-SDR …)   │         │ (GNU Radio + π/4 │            │ (Rust)          │            │ (Rust)           │
└──────────────┘         │  QPSK 復調)      │            └─────────────────┘            └──────────────────┘
                         └──────────────────┘
```

## 各段の役割

### 1. SDR + GNU Radio 受信機 (Python)

[`arib_t61_rx.py`](../../arib_t61_rx.py) は `gr-osmosdr` 経由で 10 種類の
SDR バックエンド (HackRF, RTL-SDR, Airspy R2/Mini, Airspy HF+, BladeRF,
USRP, LimeSDR, PlutoSDR, SDRplay, 汎用 Soapy) を扱えます。処理内容は
以下のとおりです。

1. RF チューニング (任意で LO オフセット — DC スパイク回避用)
2. 周波数変換 + 低域通過 + 約 4 sps へのデシメーション
3. 任意のパワースケルチ + フィードフォワード AGC
4. ルートレイズドコサイン整合フィルタ (β = 0.2)
5. 任意の FLL バンドエッジ周波数補正
6. ポリフェーズシンボルクロック同期 (4 sps → 1 sps)
7. π/4 シフト QPSK 準同期復調
   ([`pi4_qpsk_demod.py`](../../pi4_qpsk_demod.py))
8. 4 シンボル/バイトへのビット詰め

出力は公称シンボルレート 4 800 baud の 2 ビットシンボル列 (Gray 符号、
MSB 先頭) です。シンボルはファイル出力、バイト詰め出力、あるいは
ファイル記述子シンクで次段へパイプ送りできます。

### 2. フレームスライサ (Rust)

[`t61_frame_slicer`](../../src/bin/t61_frame_slicer.rs) はパックされた
シンボル列を読み込み、192 シンボルフレームの正規位置に現れる ARIB の
3 種類の同期ワード **SS1**、**S2**、**S6** を 32 ビットスライディング
ウィンドウで検出します。整列したフレームは 48 バイト (192 シンボル ×
2 ビット ÷ 8) ちょうどとして送出され、1 フレーム分のシンボルを探しても
同期を再獲得できなかった場合は全 0 バイトの「プレースホルダ」フレーム
を出力します。これで下流のタイムスタンプは 40 ms スーパーフレームに
固定したままになります。

### 3. デコーダ (Rust)

[`t61_fd_decoder`](../../src/bin/t61_fd_decoder.rs) は 48 バイトフレーム
を受け取り、フレームごとに 1 レコードを出力します。

- **`OutputMode::Json`** (既定) — 1 行 1 オブジェクトの JSONL。
  `serde_json` の `preserve_order` 機能でキーの挿入順を保持します。
- **`OutputMode::CelpOnly`** (`-c` / `--celp`) — 音声フレームに含まれる
  36 文字の CELP ペイロード (16 進文字列) のみを 1 行ずつ出力。CELP
  シンセサイザへ直接パイプ可能な形式です。

デコーダは ARIB スタックを完全に走らせます。ホワイトニング解除、
デインタリーブ、畳み込み復号 (制御チャネルは R = 1/2 K = 6、音声は
K = 9)、CRC 検査、各プロトコル固有のパーサ (ヘッダ / RICH / M フィールド、
PSC TCH、SACCH/RCH、PICH ページング、レイヤ 2 マルチフレーム再構築、
ACCH コマンド、GPS、FACCH、仙台地区バイナリ拡張) を含みます。

## なぜ 3 段独立構成か

- **リアルタイム性**: Rust バイナリは stdin/stdout を生のファイル記述子
  (`from_raw_fd(0)` / `from_raw_fd(1)`) として開くので、`read(2)` /
  `write(2)` システムコールが直接走り、Rust 側のバッファリングは
  入りません。`arib_t61_rx.py` 側では stdout に
  `blocks.file_descriptor_sink(... 1)` を、ファイルシンクには
  `set_unbuffered(True)` を使っています。これによりバッファ充填で
  詰まることなく、ライブパイプライン全体が間断なく流れます。
- **再生可能性**: 中間フォーマット (cf32 IQ、1 シンボル 1 バイト、
  パックドシンボル、48 バイトフレーム) はいずれも保存可能で、後刻の
  解析や回帰テストに使えます。
- **再現性**: ライブ SDR 入力を復号する同じ Rust バイナリで、保存済み
  の `*.t61` ファイルを決定的に復号できます。同じ入力ファイルに対して
  実行ごとに同じ出力が得られます。
- **組み合わせ自由度**: 4 シンボル/バイトのパックド出力を出すもの
  (別のデコーダ、テストベクトル生成器など) なら何でも
  `t61_frame_slicer` の入力にできます。同様に 48 バイトフレームを
  出すものは何でも `t61_fd_decoder` に流し込めます。

## ソース配置

```
arib_t61_rx.py          # GNU Radio SDR 受信機 (マルチデバイス・フロントエンド)
pi4_qpsk_demod.py       # 自作 π/4-QPSK 準同期復調ブロック
src/
├── lib.rs              # 公開再エクスポート
├── bin/
│   ├── t61_frame_slicer.rs
│   └── t61_fd_decoder.rs
├── slicer.rs           # 2 ビットシンボル列 → 48 バイトフレーム
├── decoder/            # 48 バイトフレーム → JSONL / CELP
│   ├── mod.rs          # Decoder + OutputMode + フレーム分岐
│   ├── header.rs       # RICH / PSC TCH+SACCH / PICH
│   ├── voice.rs        # CELP 音声フレーム
│   ├── acch.rs         # レイヤ 2 / RCH / SACCH / ACCH 内側フィールド
│   ├── l2_block.rs     # L2 マルチフレームブロック分岐 + FACCH
│   ├── l2_text.rs      # テキスト形式 L2 データ各種
│   ├── l2_binary.rs    # バイナリ形式 + 仙台拡張 L2 データ
│   └── gps_emit.rs     # GPS サブオブジェクト出力
├── convo.rs            # K=6 / K=9 畳み込み復号器
├── primitives.rs       # ビット演算、CRC-6/16、同期ワード比較、ホワイトニング
├── json.rs             # FieldEmitter (Cursor + serde_json::Map DSL)
├── state.rs            # DecoderState、PscState、MField
├── gps.rs              # TKY → WGS84、緯度/経度/速度/方位パーサ
└── tables.rs           # 同期ワード、ホワイトニング/インタリーブ表など
```

## ビルドと実行

```sh
# Rust 1.85+ (edition 2024)
cargo build --release

# ライブ復号 (HackRF、既定の 500 kHz LO オフセット)
PYTHONUNBUFFERED=1 python3 arib_t61_rx.py -d hackrf -f 274.60625e6 --packed-out - \
  | target/release/t61_frame_slicer \
  | target/release/t61_fd_decoder
```

詳細はそれぞれの個別ドキュメントへ。

- [SDR 受信パイプライン](sdr-pipeline.md) — GNU Radio フローグラフと
  PLL の詳細
- [フレームスライサ](frame-slicer.md) — 同期ワード探索
- [フレームデコーダ](decoder.md) — プロトコルスタック
- [JSONL / CELP 出力](json-output.md) — 出力スキーマ
- [ライブラリ API](library-api.md) — 組み込み利用
