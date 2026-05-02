#!/usr/bin/env python3
"""Multi-device SDR receiver for ARIB STD-T61 (pi/4-shifted QPSK, 4800 sym/s).

Supported backends (selected via --device): hackrf, rtlsdr, airspy, airspyhf,
bladerf, uhd (USRP), limesdr, plutosdr, sdrplay, soapy:<driver>.
"""

import argparse
import signal
import sys
import time

import numpy as np
from gnuradio import gr, blocks, filter as gr_filter, digital, analog
from gnuradio.filter import firdes

# GR 3.10 moved firdes.WIN_* to gnuradio.fft.window.
try:
    from gnuradio.fft import window as _fftwin

    WIN_HAMMING = _fftwin.WIN_HAMMING
    WIN_BLACKMAN_hARRIS = _fftwin.WIN_BLACKMAN_hARRIS
except (ImportError, AttributeError):
    WIN_HAMMING = firdes.WIN_HAMMING
    WIN_BLACKMAN_hARRIS = firdes.WIN_BLACKMAN_hARRIS

try:
    import osmosdr
except ImportError:
    print(
        "error: gr-osmosdr is required (sudo apt install gr-osmosdr)", file=sys.stderr
    )
    sys.exit(1)

from pi4_qpsk_demod import blk as Pi4QpskDemod

DEFAULT_SYM_RATE = 4800
DEFAULT_RRC_ROLLOFF = 0.2
DEFAULT_RRC_NTAPS_SYMBOLS = 11
TARGET_SPS_AT_DEMOD = 4  # 4 samples/symbol after decimation


# =====================================================================
# SDR backend definitions
# =====================================================================
class SdrBackend:
    """Per-device knowledge: native rate, gain stages, LO offset policy.

    Subclasses set class attributes and implement build_args(). The base
    class then constructs an osmosdr.source and configures it.
    """

    name = ""  # short id used by --device
    osmosdr_args = ""  # base device-id string for osmosdr
    sample_rate = 0  # native sample rate (Hz)
    decim = 0  # decimation -> reaches TARGET_SPS_AT_DEMOD
    default_lo_offset = 0  # Hz; 0 means "tune on-channel"
    gain_stages = ()  # tuple of (cli_arg_name, osmosdr_name, default, help)

    @classmethod
    def make_source(cls, args):
        src = osmosdr.source(args=cls.build_args(args))
        src.set_sample_rate(cls.sample_rate)
        src.set_center_freq(args.freq + cls.lo_offset(args), 0)
        src.set_freq_corr(args.ppm, 0)
        src.set_dc_offset_mode(0, 0)
        src.set_iq_balance_mode(0, 0)
        src.set_gain_mode(False, 0)
        for cli_name, osm_name, _default, _help in cls.gain_stages:
            value = getattr(args, cli_name)
            if value is not None:
                src.set_gain(value, osm_name, 0)
        src.set_antenna(args.antenna or "", 0)
        src.set_bandwidth(args.bandwidth or 0, 0)
        return src

    @classmethod
    def build_args(cls, args):
        if args.device_args:
            return args.device_args
        return cls.osmosdr_args

    @classmethod
    def lo_offset(cls, args):
        return args.lo_offset if args.lo_offset is not None else cls.default_lo_offset


class HackRFBackend(SdrBackend):
    name = "hackrf"
    osmosdr_args = "numchan=1 hackrf=0"
    sample_rate = 4_800_000  # 250x decim -> 19200 sps
    decim = 250
    default_lo_offset = 500_000
    gain_stages = (
        ("rf_gain", "RF", 14, "RF amp 0/14 dB"),
        ("if_gain", "IF", 24, "LNA 0..40 dB step 8"),
        ("bb_gain", "BB", 20, "VGA 0..62 dB step 2"),
    )


class RtlSdrBackend(SdrBackend):
    name = "rtlsdr"
    osmosdr_args = "numchan=1 rtl=0"
    sample_rate = 1_920_000  # 100x decim -> 19200 sps
    decim = 100
    default_lo_offset = 0  # low-IF, no DC issue
    gain_stages = (
        # RTL-SDR has a single "tuner" gain (LNA+mixer combined).
        ("tuner_gain", "TUNER", 30, "tuner gain (R820T: 0..49 in 2-3 dB steps)"),
    )


class AirspyBackend(SdrBackend):
    """Airspy R2 / Mini (NOT Airspy HF+)."""

    name = "airspy"
    osmosdr_args = "numchan=1 airspy=0"
    sample_rate = 2_500_000  # ~130x decim. Some firmware also supports 10 Msps.
    decim = 130  # -> 19230 sps; close enough, polyphase sync handles it
    default_lo_offset = 0
    gain_stages = (
        ("if_gain", "IF", 10, "Airspy LNA 0..15"),
        ("mix_gain", "MIX", 10, "Airspy mixer 0..15"),
        ("bb_gain", "BB", 10, "Airspy VGA 0..15"),
    )


class AirspyHFBackend(SdrBackend):
    name = "airspyhf"
    osmosdr_args = "numchan=1 airspyhf=0"
    sample_rate = 768_000  # 40x decim -> 19200 sps
    decim = 40
    default_lo_offset = 0
    gain_stages = ()  # AGC only; no manual gain on HF+


class BladeRFBackend(SdrBackend):
    name = "bladerf"
    osmosdr_args = "numchan=1 bladerf=0"
    sample_rate = 1_920_000  # 100x decim -> 19200 sps
    decim = 100
    default_lo_offset = 0
    gain_stages = (
        ("if_gain", "LNA", 6, "BladeRF LNA gain 0/3/6 dB"),
        ("bb_gain", "VGA1", 20, "BladeRF VGA1 5..30 dB"),
        ("bb_gain2", "VGA2", 20, "BladeRF VGA2 0..60 dB"),
    )


class UhdBackend(SdrBackend):
    name = "uhd"
    osmosdr_args = "numchan=1 uhd"
    sample_rate = 1_920_000  # 100x decim. USRP can do anything.
    decim = 100
    default_lo_offset = (
        100_000  # USRP DC removal is good but a small offset is still cheap insurance
    )
    gain_stages = (
        ("rx_gain", "", 30, "USRP RX gain (single value, device-dependent range)"),
    )


class LimeSdrBackend(SdrBackend):
    name = "limesdr"
    osmosdr_args = "numchan=1 lime=0"
    sample_rate = 1_920_000
    decim = 100
    default_lo_offset = 100_000
    gain_stages = (
        ("if_gain", "LNA", 20, "LimeSDR LNA gain 0..30"),
        ("mix_gain", "TIA", 9, "LimeSDR TIA gain 0..12"),
        ("bb_gain", "PGA", 10, "LimeSDR PGA gain -12..19"),
    )


class PlutoSdrBackend(SdrBackend):
    name = "plutosdr"
    osmosdr_args = "numchan=1 plutosdr=usb"
    sample_rate = 1_920_000
    decim = 100
    default_lo_offset = 0
    gain_stages = (("rx_gain", "", 40, "PlutoSDR RX gain -3..71 dB"),)


class SdrPlayBackend(SdrBackend):
    name = "sdrplay"
    osmosdr_args = "numchan=1 sdrplay=0"
    sample_rate = 2_000_000
    decim = 104  # 19230 sps; polyphase sync handles non-integer
    default_lo_offset = 0
    gain_stages = (("if_gain", "IF", 40, "SDRplay IF gain reduction (dB)"),)


class SoapyBackend(SdrBackend):
    """Generic SoapySDR passthrough via gr-osmosdr's soapy=... shim.

    Pass --device-args to forward arbitrary key=value parameters. Sample rate,
    decimation and a single 'rx_gain' must be set on the command line.
    """

    name = "soapy"
    osmosdr_args = "numchan=1 soapy=0"
    sample_rate = 0  # set from --sample-rate
    decim = 0  # computed at run-time
    default_lo_offset = 0
    gain_stages = (("rx_gain", "", 30, "Generic Soapy RX gain"),)


_BACKENDS = {
    cls.name: cls
    for cls in [
        HackRFBackend,
        RtlSdrBackend,
        AirspyBackend,
        AirspyHFBackend,
        BladeRFBackend,
        UhdBackend,
        LimeSdrBackend,
        PlutoSdrBackend,
        SdrPlayBackend,
        SoapyBackend,
    ]
}


# =====================================================================
# Top block
# =====================================================================
class AribT61Rx(gr.top_block):
    def __init__(self, args, backend):
        gr.top_block.__init__(self, "ARIB STD-T61 Receiver")

        self.args = args
        self.backend = backend
        self.symbol_rate = float(args.sym_rate)

        # Determine sample rate / decim. Soapy backend takes them from args.
        if backend is SoapyBackend:
            if args.sample_rate is None:
                sys.exit("--sample-rate is required for the soapy backend")
            backend.sample_rate = int(args.sample_rate)
        if backend.decim == 0 or args.sample_rate is not None:
            # Recompute decim to land near 4 sps at the symbol rate.
            target_post_decim = TARGET_SPS_AT_DEMOD * self.symbol_rate
            backend.decim = max(1, round(backend.sample_rate / target_post_decim))

        self.sample_rate = backend.sample_rate
        self.decim = backend.decim
        self.sps_after_decim = self.sample_rate / self.decim / self.symbol_rate
        if abs(self.sps_after_decim - round(self.sps_after_decim)) > 1e-3:
            print(
                f"note: non-integer sps after decim "
                f"({self.sps_after_decim:.4f}); polyphase sync handles it.",
                file=sys.stderr,
            )

        self.lo_offset = float(backend.lo_offset(args))

        # ---- Source -----------------------------------------------------
        self.src = backend.make_source(args)

        # ---- Frequency translation + LPF + decimation -------------------
        cutoff = self.symbol_rate * (1.0 + DEFAULT_RRC_ROLLOFF) * 0.55
        transition = self.symbol_rate * 0.5
        lp_taps = firdes.low_pass(
            1.0, self.sample_rate, cutoff, transition, WIN_HAMMING
        )
        self.xlate = gr_filter.freq_xlating_fir_filter_ccc(
            self.decim, lp_taps, -self.lo_offset, self.sample_rate
        )

        # ---- Optional pre-AGC squelch -----------------------------------
        self.squelch = analog.pwr_squelch_cc(args.squelch_db, 1e-3, 0, False)

        # ---- Feedforward AGC --------------------------------------------
        self.agc = analog.feedforward_agc_cc(64, 1.0)

        # ---- RRC matched filter -----------------------------------------
        sps = round(self.sample_rate / self.decim / self.symbol_rate)
        rrc_taps = firdes.root_raised_cosine(
            1.0, sps, 1.0, DEFAULT_RRC_ROLLOFF, DEFAULT_RRC_NTAPS_SYMBOLS * sps
        )
        self.rrc = gr_filter.fir_filter_ccf(1, rrc_taps)

        # ---- Optional FLL -----------------------------------------------
        self.fll = None
        if args.fll_bw > 0:
            self.fll = digital.fll_band_edge_cc(
                sps, DEFAULT_RRC_ROLLOFF, DEFAULT_RRC_NTAPS_SYMBOLS * sps, args.fll_bw
            )

        # ---- Polyphase symbol clock recovery ----------------------------
        nfilt = 32
        timing_taps = firdes.root_raised_cosine(
            nfilt,
            nfilt * sps,
            1.0,
            DEFAULT_RRC_ROLLOFF,
            nfilt * DEFAULT_RRC_NTAPS_SYMBOLS,
        )
        self.clock_sync = digital.pfb_clock_sync_ccf(
            sps,
            args.timing_loop_bw,
            timing_taps,
            nfilt,
            nfilt // 2,
            args.timing_max_dev,
            1,
        )

        # ---- pi/4-QPSK quasi-coherent demod -----------------------------
        self.demod = Pi4QpskDemod(
            loop_gain=args.phase_loop_gain,
            freq_loop_gain=args.freq_loop_gain,
            gray_coded=True,
            msb_first=True,
        )

        chain = [self.src, self.xlate, self.squelch, self.agc, self.rrc]
        if self.fll is not None:
            chain.append(self.fll)
        chain.extend([self.clock_sync, self.demod])
        self.connect(*chain)

        # ---- Output sinks -----------------------------------------------
        demod_drained = False

        if args.bits_out:
            self.unpack_bits = blocks.unpack_k_bits_bb(2)
            self.bits_sink = blocks.file_sink(gr.sizeof_char, args.bits_out, False)
            self.bits_sink.set_unbuffered(True)
            self.connect(self.demod, self.unpack_bits, self.bits_sink)
            demod_drained = True

        if args.packed_out:
            self.unpack_for_pack = blocks.unpack_k_bits_bb(2)
            self.pack8 = blocks.pack_k_bits_bb(8)
            if args.packed_out == "-":
                self.packed_sink = blocks.file_descriptor_sink(gr.sizeof_char, 1)
            else:
                self.packed_sink = blocks.file_sink(
                    gr.sizeof_char, args.packed_out, False
                )
                self.packed_sink.set_unbuffered(True)
            self.connect(self.demod, self.unpack_for_pack, self.pack8, self.packed_sink)
            demod_drained = True

        if not demod_drained:
            self.null_sink = blocks.null_sink(gr.sizeof_char)
            self.connect(self.demod, self.null_sink)

        if args.iq_out:
            self.iq_sink = blocks.file_sink(gr.sizeof_gr_complex, args.iq_out, False)
            self.iq_sink.set_unbuffered(True)
            self.connect(self.xlate, self.iq_sink)

        if args.gui:
            self._setup_gui()

    def _setup_gui(self):
        try:
            from PyQt5 import Qt
            from gnuradio import qtgui
            import sip
        except ImportError as e:
            print(
                f"warning: --gui requested but PyQt5/qtgui unavailable ({e})",
                file=sys.stderr,
            )
            return

        self.qapp = Qt.QApplication.instance() or Qt.QApplication(sys.argv)
        self.win = Qt.QWidget()
        self.win.setWindowTitle(f"ARIB STD-T61 RX ({self.backend.name})")
        self.layout = Qt.QVBoxLayout(self.win)

        sr_after = self.sample_rate / self.decim

        self.fft = qtgui.freq_sink_c(
            1024,
            WIN_BLACKMAN_hARRIS,
            self.args.freq + self.lo_offset,
            self.sample_rate,
            f"Spectrum (raw {self.backend.name})",
            1,
            None,
        )
        self.fft.set_y_axis(-120, 0)
        self.layout.addWidget(sip.wrapinstance(self.fft.qwidget(), Qt.QWidget))
        self.connect(self.src, self.fft)

        self.wf = qtgui.waterfall_sink_c(
            1024,
            WIN_BLACKMAN_hARRIS,
            self.args.freq,
            sr_after,
            "Channel waterfall (post-xlate, baseband)",
            1,
            None,
        )
        self.wf.set_intensity_range(-120, 0)
        self.layout.addWidget(sip.wrapinstance(self.wf.qwidget(), Qt.QWidget))
        self.connect(self.xlate, self.wf)

        self.cons = qtgui.const_sink_c(1024, "Constellation (post-sync)", 1, None)
        self.cons.set_y_axis(-2, 2)
        self.cons.set_x_axis(-2, 2)
        self.layout.addWidget(sip.wrapinstance(self.cons.qwidget(), Qt.QWidget))
        self.connect(self.clock_sync, self.cons)

        self.win.resize(1000, 800)
        self.win.show()


# =====================================================================
# CLI
# =====================================================================
def parse_args():
    p = argparse.ArgumentParser(
        description="Multi-device receiver for ARIB STD-T61 (pi/4-QPSK)"
    )
    p.add_argument(
        "--device",
        "-d",
        required=True,
        choices=sorted(_BACKENDS.keys()),
        help="SDR backend (required)",
    )
    p.add_argument(
        "--device-args",
        default=None,
        help="Override osmosdr device args (e.g. 'rtl=1', " "'soapy=0,driver=lime')",
    )
    p.add_argument(
        "--freq",
        "-f",
        type=float,
        required=True,
        help="RF center frequency in Hz (e.g. 467.000e6)",
    )
    p.add_argument(
        "--sample-rate",
        type=float,
        default=None,
        help="Override SDR sample rate in Hz (default: device-specific)",
    )
    p.add_argument(
        "--bandwidth",
        type=float,
        default=None,
        help="Front-end bandwidth in Hz (default: auto)",
    )
    p.add_argument(
        "--antenna", default=None, help="Antenna port name (device-specific)"
    )
    p.add_argument(
        "--sym-rate",
        type=float,
        default=DEFAULT_SYM_RATE,
        help=f"Symbol rate in sym/s (default {DEFAULT_SYM_RATE})",
    )
    p.add_argument(
        "--ppm", type=int, default=0, help="Frequency correction in ppm (default 0)"
    )
    p.add_argument(
        "--lo-offset",
        type=float,
        default=None,
        help="LO offset in Hz; default is device-specific "
        "(500 kHz for HackRF, 0 for low-IF devices)",
    )

    # Per-device gain stages: register every name found across backends.
    # Each backend uses whichever subset applies; unset values are skipped.
    seen = {}
    for back in _BACKENDS.values():
        for cli_name, osm_name, default, helptxt in back.gain_stages:
            if cli_name in seen:
                continue
            seen[cli_name] = True
            p.add_argument(
                f"--{cli_name.replace('_', '-')}",
                type=int,
                default=default,
                help=f"{helptxt} (default {default})",
            )

    p.add_argument(
        "--phase-loop-gain", type=float, default=0.2, help="PLL alpha (default 0.2)"
    )
    p.add_argument(
        "--freq-loop-gain",
        type=float,
        default=0.01,
        help="PLL beta, 2nd-order frequency integrator (default 0.01)",
    )
    p.add_argument(
        "--timing-loop-bw",
        type=float,
        default=0.02,
        help="Polyphase clock-sync loop bandwidth (default 0.02)",
    )
    p.add_argument(
        "--timing-max-dev",
        type=float,
        default=1.5,
        help="Polyphase clock-sync max rate deviation (default 1.5)",
    )
    p.add_argument(
        "--squelch-db",
        type=float,
        default=-100.0,
        help="Power-squelch threshold in dB (default -100 = off)",
    )
    p.add_argument(
        "--fll-bw",
        type=float,
        default=0.0,
        help="FLL band-edge bandwidth (default 0 = off)",
    )
    p.add_argument(
        "--bits-out", type=str, default=None, help="Write bits (1/byte) to file"
    )
    p.add_argument(
        "--packed-out",
        type=str,
        default=None,
        help="Write packed bytes (4 sym/byte MSB-first); '-' for stdout",
    )
    p.add_argument(
        "--iq-out", type=str, default=None, help="Save decimated IQ (complex64) to file"
    )
    p.add_argument(
        "--gui",
        action="store_true",
        help="Open Qt GUI (spectrum / waterfall / constellation)",
    )
    p.add_argument(
        "--duration",
        type=float,
        default=0.0,
        help="Run for N seconds then exit (0 = until Ctrl-C)",
    )
    args = p.parse_args()

    # Set unused gain stages to None so make_source can skip them.
    backend = _BACKENDS[args.device]
    used_stages = {cli_name for cli_name, *_ in backend.gain_stages}
    for cli_name in seen:
        if cli_name not in used_stages:
            setattr(args, cli_name, None)
    return args, backend


def main():
    args, backend = parse_args()
    log = lambda *a, **kw: print(*a, file=sys.stderr, **kw)

    log(f"ARIB STD-T61 receiver.")
    log(f"Device      : {backend.name}")
    log(
        f"Center freq : {args.freq/1e6:.4f} MHz "
        f"(LO offset {backend.lo_offset(args)/1e3:+.1f} kHz)"
    )
    rate = args.sample_rate if args.sample_rate is not None else backend.sample_rate
    log(f"Sample rate : {rate/1e6:.4f} Msps")
    log(f"Phase loop  : alpha = {args.phase_loop_gain}, beta = {args.freq_loop_gain}")
    log(f"Timing loop : BW = {args.timing_loop_bw}, max_dev = {args.timing_max_dev}")
    log(f"Squelch     : {args.squelch_db} dB")
    log(f"FLL         : {'BW = ' + str(args.fll_bw) if args.fll_bw > 0 else 'off'}")

    tb = AribT61Rx(args, backend)

    def stop(signum, frame):
        log("\nStopping...")
        tb.stop()
        tb.wait()
        sys.exit(0)

    signal.signal(signal.SIGINT, stop)

    if args.gui:
        tb.start()
        try:
            tb.qapp.exec_()
        finally:
            tb.stop()
            tb.wait()
    else:
        tb.start()
        if args.duration > 0:
            time.sleep(args.duration)
            tb.stop()
            tb.wait()
        else:
            log("Running. Ctrl-C to stop.")
            try:
                tb.wait()
            except KeyboardInterrupt:
                tb.stop()
                tb.wait()


if __name__ == "__main__":
    main()
