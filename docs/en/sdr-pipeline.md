# SDR receiver pipeline

The receiver is composed of two Python files:

- [`arib_t61_rx.py`](../../arib_t61_rx.py) — GNU Radio top block,
  per-SDR backend definitions, and CLI.
- [`pi4_qpsk_demod.py`](../../pi4_qpsk_demod.py) — custom
  `gr.sync_block` implementing the JPH06132996A π/4-shifted QPSK
  quasi-coherent demodulator.

## Channel parameters

| Parameter | Value |
|---|---|
| Modulation | π/4-shifted DQPSK |
| Symbol rate | 4 800 sym/s (Gray-coded, 2 bits/symbol → 9 600 b/s) |
| Pulse shape | Root-raised cosine, β = 0.2, ±5.5 symbols span |
| Frame | 192 symbols (40 ms super-frame, 48 packed bytes) |

The ARIB downlink is SCPC/FDMA. Each carrier carries one channel; the
receiver does not need to demultiplex multiple sub-channels.

## Backend abstraction

Every supported SDR is a subclass of `SdrBackend` defined near the top
of `arib_t61_rx.py`. The base class encodes shared behaviour
(`make_source`, `lo_offset`, `build_args`); each subclass declares:

```python
name              # short id used by --device
osmosdr_args      # base device-id string for gr-osmosdr
sample_rate       # native rate in Hz
decim             # decimation that lands near 4 sps after the front end
default_lo_offset # 0 means tune on-channel
gain_stages       # tuple of (cli_arg_name, osmosdr_name, default, help)
```

Defaults at a glance (see [arib_t61_rx.py:88-206](../../arib_t61_rx.py#L88-L206)):

| Backend | Sample rate | Decim | LO offset | Notes |
|---|---|---|---|---|
| `hackrf` | 4.8 Msps | 250 | +500 kHz | DC-coupled tuner; offset avoids spike on channel |
| `rtlsdr` | 1.92 Msps | 100 | 0 | low-IF, no DC issue |
| `airspy` | 2.5 Msps | 130 | 0 | non-integer 19.23 ksps; polyphase sync absorbs it |
| `airspyhf` | 768 ksps | 40 | 0 | AGC only, no manual gain stages |
| `bladerf` | 1.92 Msps | 100 | 0 | LNA + VGA1 + VGA2 |
| `uhd` | 1.92 Msps | 100 | +100 kHz | small offset is cheap insurance |
| `limesdr` | 1.92 Msps | 100 | +100 kHz | LNA + TIA + PGA |
| `plutosdr` | 1.92 Msps | 100 | 0 | single rx_gain |
| `sdrplay` | 2.0 Msps | 104 | 0 | non-integer; polyphase sync absorbs it |
| `soapy` | from CLI | computed | 0 | passes `--device-args` straight through |

`SoapyBackend` and any device that ships with `decim == 0` recompute
decimation at runtime so the post-decimation rate lands near
`TARGET_SPS_AT_DEMOD * symbol_rate` (= 19 200 sps with the defaults).
Non-integer ratios are tolerated by the polyphase clock-sync block.

## Flowgraph

```
src ─▶ xlate ─▶ squelch ─▶ agc ─▶ rrc ─▶ [fll] ─▶ clock_sync ─▶ demod ─▶ pack ─▶ sink(s)
                                                            │
                                                            ├─▶ bits_sink (1 sym/byte)
                                                            ├─▶ packed_sink (4 sym/byte)
                                                            └─▶ iq_sink (decimated cf32)
```

Stages, with the GNU Radio block in parentheses:

1. **Source** (`osmosdr.source`). Antenna, bandwidth, gain stages and
   PPM correction are applied per backend.
2. **Frequency translation + LPF + decimation**
   (`gr_filter.freq_xlating_fir_filter_ccc`). Cutoff is
   `symbol_rate × (1 + β) × 0.55`, transition `symbol_rate × 0.5`,
   Hamming window (see
   [arib_t61_rx.py:263-270](../../arib_t61_rx.py#L263-L270)).
3. **Squelch** (`analog.pwr_squelch_cc`). Default −100 dB (off).
   Useful on noisy receive sites where input bursts otherwise drag
   the AGC.
4. **Feedforward AGC** (`analog.feedforward_agc_cc`, window 64,
   reference 1.0).
5. **Root-raised-cosine matched filter**
   (`gr_filter.fir_filter_ccf`). Same β and span as the transmitter
   pulse shaping.
6. **Optional FLL** (`digital.fll_band_edge_cc`). Off by default; turn
   on for captures with significant residual frequency error after
   the LO offset.
7. **Polyphase clock recovery** (`digital.pfb_clock_sync_ccf`). 32
   filter banks, RRC taps with 32 × N = 32 × 11 symbols, default
   `timing_loop_bw = 0.02`, `timing_max_dev = 1.5`.
8. **Demodulator** (`Pi4QpskDemod`, see below).
9. **Packing** (`blocks.unpack_k_bits_bb(2)` →
   `blocks.pack_k_bits_bb(8)`). Two-bit dibits are packed back into
   bytes MSB first so the slicer sees four symbols per byte.

### Sinks

Any combination of three sinks can be enabled:

- `--bits-out PATH` — one symbol (0..3) per byte. Useful for
  inspection or piping into custom tools.
- `--packed-out PATH` (or `-` for stdout) — 4 symbols per byte,
  MSB-first. This is the format `t61_frame_slicer` consumes.
  Stdout uses `blocks.file_descriptor_sink(gr.sizeof_char, 1)` so
  output is unbuffered.
- `--iq-out PATH` — decimated baseband IQ (`complex64`) at the
  output of `xlate`. Saves the captured channel post-decimation,
  ready for offline replay.

If neither `--bits-out` nor `--packed-out` is present, the demod is
drained into a `null_sink` to keep the flowgraph alive.

### Optional Qt GUI

`--gui` adds three sinks for visual inspection:

- Frequency sink on the raw input (FFT, Blackman-Harris window).
- Waterfall on the post-`xlate` baseband.
- Constellation sink on the post-`clock_sync` symbol stream.

The constellation sink shows π/4-shifted QPSK rotating between two
4-point constellations every symbol — that "8-point pinwheel" is the
expected signature.

## π/4 QPSK quasi-coherent demodulator

The demodulator implements the architecture of Japanese patent
JPH06132996A: a 2nd-order PLL drives a phase rotation (φ = δ + m·π/4),
and the rotated symbol is compared against a quadrant decision to
produce both an error term for the loop and the symbol output.

Block parameters (in `pi4_qpsk_demod.py`):

| Parameter | Default | Role |
|---|---|---|
| `loop_gain` (α) | 0.05 | 1st-order phase loop gain |
| `freq_loop_gain` (β) | α² / 4 | 2nd-order frequency integrator (critical damping) |
| `lock_iir_alpha` | 0.02 | IIR α for the running |err| lock detector |
| `unlock_threshold` | 0.5 | running |err| threshold above which we declare unlock |
| `unlock_reset_after` | 300 | sustained unlocked symbols → full state reset |
| `gray_coded` | True | output dibit is Gray-mapped |
| `msb_first` | True | MSB of the dibit comes first |

### Per-sample loop

For each input sample (1 sample/symbol after `clock_sync`):

```
phi  = delta + m·(π/4)              # rotation
i2   = i1·cos(phi) + q1·sin(phi)
q2   = -i1·sin(phi) + q1·cos(phi)
ic   = (q2 < 0) ? -i2 : i2          # patent eq (c)
qc   = (i2 < 0) ? -q2 : q2
err  = ic - qc

err_running = (1-α_iir)·err_running + α_iir·|err|
nu          = nu - β·err
delta       = delta - α·err + nu
```

Two extra paths exist:

- **Squelch fast path** (mag² < 1e-12): output 0, full state reset
  (delta = nu = err_running = unlock_count = 0). Lets the receiver
  recover quickly from intentional zero stuffing (`pwr_squelch_cc`).
- **Periodic unlock reset**: when `err_running` stays above
  `unlock_threshold` for `unlock_reset_after` symbols, δ and ν are
  zeroed. Prevents the loop from camping on a metastable lock.

### Differential decoding

After the quadrant decision (CCW from +I axis), the dibit is recovered
from the *change* in quadrant between the previous and current
symbols:

```python
dq    = (qd - prev_q) % 4
dibit = _DQ_TO_DIBIT[dq]    # [+π/4=0, +3π/4=1, -3π/4=3, -π/4=2]
```

This is what makes the demodulator differential (the "D" in DQPSK):
absolute phase doesn't matter, only successive transitions do. While
unlocked the first dibit per re-lock is dropped (output 0 with
`have_prev = False`).

`gray_coded` and `msb_first` flags allow toggling those last two
post-processing steps in tests; production wiring uses both ON.

## Tuning

For hard captures the knobs that matter most are:

- `--phase-loop-gain` / `--freq-loop-gain` — the 2nd-order PLL gains.
  Defaults of 0.2 / 0.01 are intentionally wide; reduce if the
  captured signal is high-SNR but there's residual phase noise.
- `--timing-loop-bw` / `--timing-max-dev` — polyphase clock-sync. If
  symbol rate offset is large, raise `--timing-max-dev`.
- `--squelch-db` — clamp the AGC during silence.
- `--fll-bw` — 0 by default. Enable on captures where the LO error is
  comparable to or exceeds the symbol rate.
- `--lo-offset` — defaults are device-specific. Override for SDRs
  with unusual DC-spike behaviour.
