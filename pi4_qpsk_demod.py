"""pi/4-shifted QPSK quasi-coherent demodulator (JPH06132996A architecture).

In: complex64, 1 sample/symbol. Out: uint8 dibit (0..3).
"""

import numpy as np
from gnuradio import gr

# dq -> data dibit (Gray, MSB-first): +pi/4=0, +3pi/4=1, -3pi/4=3, -pi/4=2
_DQ_TO_DIBIT = np.array([0, 1, 3, 2], dtype=np.uint8)


class blk(gr.sync_block):
    def __init__(
        self,
        loop_gain=0.05,
        freq_loop_gain=None,
        lock_iir_alpha=0.02,
        unlock_threshold=0.5,
        unlock_reset_after=300,
        gray_coded=True,
        msb_first=True,
    ):
        gr.sync_block.__init__(
            self,
            name="pi/4-QPSK Quasi-Coherent Demod",
            in_sig=[np.complex64],
            out_sig=[np.uint8],
        )
        self.alpha = float(loop_gain)
        # 2nd-order PLL: critically damped if beta = alpha^2 / 4.
        self.beta = (
            self.alpha * self.alpha / 4.0
            if freq_loop_gain is None
            else float(freq_loop_gain)
        )
        self.gray_coded = bool(gray_coded)
        self.msb_first = bool(msb_first)

        # Lock detector: |err| ~0.2 locked, ~1.0 unlocked.
        self.lock_iir_alpha = float(lock_iir_alpha)
        self.unlock_threshold = float(unlock_threshold)
        self.unlock_reset_after = int(unlock_reset_after)

        self.delta = 0.0
        self.nu = 0.0
        self.m = 0
        self.prev_q = 0
        self.have_prev = False
        self.err_running = 0.0
        self.unlock_count = 0

    def work(self, input_items, output_items):
        in0 = input_items[0]
        out = output_items[0]
        n = len(in0)
        if n == 0:
            return 0

        I1 = in0.real
        Q1 = in0.imag

        delta = self.delta
        nu = self.nu
        m = self.m
        alpha = self.alpha
        beta = self.beta
        err_running = self.err_running
        unlock_count = self.unlock_count
        lock_iir_alpha = self.lock_iir_alpha
        unlock_threshold = self.unlock_threshold
        unlock_reset_after = self.unlock_reset_after
        quarter = np.pi / 4.0

        for k in range(n):
            phi = delta + m * quarter
            c = np.cos(phi)
            s = np.sin(phi)

            i1 = float(I1[k])
            q1 = float(Q1[k])

            # Squelch fast path: full state reset on exact zeros.
            mag2 = i1 * i1 + q1 * q1
            if mag2 < 1e-12:
                out[k] = 0
                self.have_prev = False
                delta = 0.0
                nu = 0.0
                err_running = 0.0
                unlock_count = 0
                m = (m + 1) % 8
                continue

            # Unit-circle normalisation -> err independent of amplitude.
            inv = 1.0 / np.sqrt(mag2)
            i1 *= inv
            q1 *= inv

            i2 = i1 * c + q1 * s
            q2 = -i1 * s + q1 * c

            # Phase comparator (patent eq (c)).
            ic = -i2 if q2 < 0.0 else i2
            qc = -q2 if i2 < 0.0 else q2
            err = ic - qc

            err_running = (1.0 - lock_iir_alpha) * err_running + lock_iir_alpha * abs(
                err
            )
            unlocked = err_running > unlock_threshold

            # Sustained unlock -> periodic state reset.
            if unlocked:
                unlock_count += 1
                if unlock_count > unlock_reset_after:
                    delta = 0.0
                    nu = 0.0
                    unlock_count = 0
            else:
                unlock_count = 0

            # 2nd-order PLL update (always; output gated separately).
            nu = nu - beta * err
            delta = delta - alpha * err + nu

            # Quadrant decision (CCW from +I axis).
            if i2 >= 0.0:
                qd = 0 if q2 >= 0.0 else 3
            else:
                qd = 1 if q2 >= 0.0 else 2

            if unlocked:
                out[k] = 0
                self.have_prev = False
                m = (m + 1) % 8
                continue

            if self.have_prev:
                dq = (qd - self.prev_q) % 4
                dibit = int(_DQ_TO_DIBIT[dq])
            else:
                dibit = 0
                self.have_prev = True
            self.prev_q = qd

            if not self.gray_coded:
                b1 = (dibit >> 1) & 1
                b0 = dibit & 1
                if b1 == 1:
                    b0 ^= 1
                dibit = (b1 << 1) | b0
            if not self.msb_first:
                b1 = (dibit >> 1) & 1
                b0 = dibit & 1
                dibit = (b0 << 1) | b1

            out[k] = np.uint8(dibit)
            m = (m + 1) % 8

        self.delta = delta
        self.nu = nu
        self.m = m
        self.err_running = err_running
        self.unlock_count = unlock_count
        return n
