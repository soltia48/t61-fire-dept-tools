# SDR 受信パイプライン

受信機は 2 つの Python ファイルで構成されます。

- [`arib_t61_rx.py`](../../arib_t61_rx.py) — GNU Radio のトップブロッ
  ク、SDR バックエンド定義、CLI。
- [`pi4_qpsk_demod.py`](../../pi4_qpsk_demod.py) — 自作の `gr.sync_block`。
  特開平 06-132996 の方式に基づく π/4 シフト QPSK 準同期復調器を実装。

## チャネルパラメータ

| パラメータ | 値 |
|---|---|
| 変調方式 | π/4 シフト DQPSK |
| シンボルレート | 4 800 sym/s (Gray 符号、2 ビット/シンボル → 9 600 b/s) |
| パルス整形 | ルートレイズドコサイン、β = 0.2、±5.5 シンボル長 |
| フレーム | 192 シンボル (40 ms スーパーフレーム、48 パックドバイト) |

ARIB の下り回線は SCPC/FDMA 方式であり、各搬送波が 1 チャネルを担い
ます。受信機側で副チャネル分離は不要です。

## バックエンド抽象化

サポート対象の SDR は `arib_t61_rx.py` 冒頭で定義された `SdrBackend`
の派生クラスとして表現されます。共通動作 (`make_source`、
`lo_offset`、`build_args`) は基底クラスに、各派生は次の属性を宣言する
だけです。

```python
name              # --device で使う短い識別子
osmosdr_args      # gr-osmosdr 用のデバイス ID 文字列
sample_rate       # ネイティブサンプリングレート (Hz)
decim             # フロントエンド後に約 4 sps となるデシメーション値
default_lo_offset # 0 ならオンチャネル
gain_stages       # (CLI 引数名, osmosdr 名, 既定値, ヘルプ) のタプル
```

主な既定値 (詳細は
[arib_t61_rx.py:88-206](../../arib_t61_rx.py#L88-L206)):

| バックエンド | サンプルレート | Decim | LO オフセット | 備考 |
|---|---|---|---|---|
| `hackrf` | 4.8 Msps | 250 | +500 kHz | DC 結合チューナ用にオフセット |
| `rtlsdr` | 1.92 Msps | 100 | 0 | low-IF、DC 問題なし |
| `airspy` | 2.5 Msps | 130 | 0 | 19.23 ksps の非整数比、ポリフェーズが吸収 |
| `airspyhf` | 768 ksps | 40 | 0 | AGC のみ、手動ゲインなし |
| `bladerf` | 1.92 Msps | 100 | 0 | LNA + VGA1 + VGA2 |
| `uhd` | 1.92 Msps | 100 | +100 kHz | 念のため小さなオフセット |
| `limesdr` | 1.92 Msps | 100 | +100 kHz | LNA + TIA + PGA |
| `plutosdr` | 1.92 Msps | 100 | 0 | rx_gain 単一 |
| `sdrplay` | 2.0 Msps | 104 | 0 | 非整数比、ポリフェーズが吸収 |
| `soapy` | CLI 指定 | 計算 | 0 | `--device-args` を素通し |

`SoapyBackend` および `decim == 0` のデバイスは、デシメーション後の
レートが `TARGET_SPS_AT_DEMOD * symbol_rate` (= 既定で 19 200 sps) に
近づくよう実行時に decim を再計算します。非整数比は後段のポリフェーズ
クロック同期が吸収します。

## フローグラフ

```
src ─▶ xlate ─▶ squelch ─▶ agc ─▶ rrc ─▶ [fll] ─▶ clock_sync ─▶ demod ─▶ pack ─▶ シンク群
                                                            │
                                                            ├─▶ bits_sink (1 シンボル/バイト)
                                                            ├─▶ packed_sink (4 シンボル/バイト)
                                                            └─▶ iq_sink (デシメーション後 cf32)
```

各段の役割 (括弧内は GNU Radio ブロック):

1. **ソース** (`osmosdr.source`)。アンテナ、帯域幅、ゲインステージ、
   PPM 補正をバックエンド毎に設定。
2. **周波数変換 + LPF + デシメーション**
   (`gr_filter.freq_xlating_fir_filter_ccc`)。カットオフは
   `symbol_rate × (1 + β) × 0.55`、遷移帯は `symbol_rate × 0.5`、
   Hamming 窓 (
   [arib_t61_rx.py:263-270](../../arib_t61_rx.py#L263-L270))。
3. **スケルチ** (`analog.pwr_squelch_cc`)。既定 −100 dB (実質無効)。
   ノイズ環境で AGC が暴れる場合に有効。
4. **フィードフォワード AGC** (`analog.feedforward_agc_cc`、ウィンドウ
   64、参照 1.0)。
5. **RRC 整合フィルタ** (`gr_filter.fir_filter_ccf`)。送信側パルス整形
   と同じ β とスパン。
6. **任意 FLL** (`digital.fll_band_edge_cc`)。既定オフ。LO オフセット
   後にも残留周波数誤差が大きい場合に有効化。
7. **ポリフェーズクロック同期** (`digital.pfb_clock_sync_ccf`)。フィル
   タバンク数 32、RRC タップ 32 × 11 シンボル、`timing_loop_bw =
   0.02`、`timing_max_dev = 1.5` (既定)。
8. **復調** (`Pi4QpskDemod`、後述)。
9. **パッキング** (`blocks.unpack_k_bits_bb(2)` →
   `blocks.pack_k_bits_bb(8)`)。2 ビットの dibit を MSB 先頭で再度
   バイトに詰め、スライサが 4 シンボル/バイトとして読めるようにする。

### シンク

最大 3 系統まで併用可能です。

- `--bits-out PATH` — 1 バイトに 1 シンボル (0..3)。検査やカスタム
  ツールへのパイプ用。
- `--packed-out PATH` (`-` で stdout) — 4 シンボル/バイト、MSB 先頭。
  `t61_frame_slicer` の入力フォーマット。stdout には
  `blocks.file_descriptor_sink(gr.sizeof_char, 1)` を使い、無バッファ
  出力。
- `--iq-out PATH` — `xlate` 出力のデシメーション後ベースバンド IQ
  (`complex64`)。後刻のオフラインリプレイ用。

`--bits-out` も `--packed-out` も指定しなければ復調出力は `null_sink`
へドレインされ、フローグラフが回り続けます。

### 任意の Qt GUI

`--gui` を付けると 3 種類のシンクが追加され、目視診断ができます。

- 入力スペクトル (FFT、Blackman-Harris 窓)。
- `xlate` 後ベースバンドのウォーターフォール。
- `clock_sync` 後シンボル列のコンスタレーション。

コンスタレーションは π/4 シフト QPSK の特性により、シンボル毎に
2 種類の 4 点が交互に現れて「8 点風車」のようなパターンとして見えます。

## π/4 QPSK 準同期復調器

特開平 06-132996 のアーキテクチャを実装。2 次 PLL が位相回転 (φ = δ +
m·π/4) を駆動し、回転後シンボルを象限判定して PLL 用誤差とシンボル
出力の双方を生成します。

ブロック引数 (`pi4_qpsk_demod.py`):

| 引数 | 既定 | 役割 |
|---|---|---|
| `loop_gain` (α) | 0.05 | 1 次位相ループゲイン |
| `freq_loop_gain` (β) | α² / 4 | 2 次周波数積分項 (臨界制動) |
| `lock_iir_alpha` | 0.02 | \|err\| 走行平均の IIR α |
| `unlock_threshold` | 0.5 | \|err\| 走行平均がこの値超でアンロック宣言 |
| `unlock_reset_after` | 300 | 連続アンロック超過で全状態リセット |
| `gray_coded` | True | 出力 dibit を Gray マッピング |
| `msb_first` | True | dibit の MSB を先に出力 |

### サンプル毎ループ

(`clock_sync` 後は 1 サンプル/シンボル)

```
phi  = delta + m·(π/4)
i2   = i1·cos(phi) + q1·sin(phi)
q2   = -i1·sin(phi) + q1·cos(phi)
ic   = (q2 < 0) ? -i2 : i2          # 特許 (c) 式
qc   = (i2 < 0) ? -q2 : q2
err  = ic - qc

err_running = (1-α_iir)·err_running + α_iir·|err|
nu          = nu - β·err
delta       = delta - α·err + nu
```

特殊経路:

- **スケルチ高速路** (mag² < 1e-12): 出力 0、状態を全リセット
  (delta = nu = err_running = unlock_count = 0)。`pwr_squelch_cc`
  が 0 を吐いた直後に PLL がすばやく復帰するための処理。
- **定期アンロックリセット**: `err_running` が `unlock_threshold` 超を
  `unlock_reset_after` シンボル続けた場合 δ・ν をゼロ化。準安定ロック
  への居座りを防ぎます。

### 差動復号

象限判定 (CCW、+I 軸基準) のあと、前後シンボル間の象限差分から dibit
を得ます。

```python
dq    = (qd - prev_q) % 4
dibit = _DQ_TO_DIBIT[dq]    # [+π/4=0, +3π/4=1, -3π/4=3, -π/4=2]
```

これが DQPSK の "D" 部分。絶対位相は無関係で、連続するシンボル間の
位相変化のみが信号となります。アンロック復帰直後の最初の dibit は
`have_prev = False` 経由で破棄 (出力 0) されます。

`gray_coded` と `msb_first` フラグで最後 2 段の後処理を切り替え可能で、
テスト用です。実運用では両方 ON。

## チューニング

難しい受信環境では次のつまみが効きます。

- `--phase-loop-gain` / `--freq-loop-gain` — 2 次 PLL のゲイン。既定
  0.2 / 0.01 は意図的にやや広め。高 SNR で位相雑音が残るときは下げる。
- `--timing-loop-bw` / `--timing-max-dev` — ポリフェーズクロック同期。
  シンボルレート誤差が大きいときは `--timing-max-dev` を増やす。
- `--squelch-db` — 無音時に AGC を抑える。
- `--fll-bw` — 既定 0。LO 誤差がシンボルレート相当か超える場合に有効化。
- `--lo-offset` — 既定はデバイス毎。DC スパイク特性が異常な SDR では
  オーバーライド。
