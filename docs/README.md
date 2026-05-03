# t61-fire-dept-tools — Technical documentation

This directory contains technical documentation for the
`t61-fire-dept-tools` repository, covering the SDR receiver, the
frame slicer, the decoder, and the on-the-wire / on-the-pipe formats
used between them.

## English

- [Architecture overview](en/architecture.md)
- [SDR receiver pipeline (`arib_t61_rx.py` / `pi4_qpsk_demod.py`)](en/sdr-pipeline.md)
- [Frame slicer (`t61_frame_slicer`)](en/frame-slicer.md)
- [Frame decoder (`t61_fd_decoder`)](en/decoder.md)
- [JSONL / CELP output reference](en/json-output.md)
- [Library API (`t61_fd` crate)](en/library-api.md)

## 日本語

- [アーキテクチャ概観](ja/architecture.md)
- [SDR 受信パイプライン (`arib_t61_rx.py` / `pi4_qpsk_demod.py`)](ja/sdr-pipeline.md)
- [フレームスライサ (`t61_frame_slicer`)](ja/frame-slicer.md)
- [フレームデコーダ (`t61_fd_decoder`)](ja/decoder.md)
- [JSONL / CELP 出力リファレンス](ja/json-output.md)
- [ライブラリ API (`t61_fd` crate)](ja/library-api.md)
