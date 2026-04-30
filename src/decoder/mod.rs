//! High-level frame processing.  Public entry point is [`Decoder`].
//!
//! Each frame is built up as a `serde_json::Map<String, Value>` and
//! serialized once at the end of [`Decoder::process_frame`], so the
//! per-field code is plain insert calls with no I/O `?` propagation.
//!
//! Submodule layout:
//!
//! * [`header`] — RICH + PSC TCH/SACCH + PICH
//! * [`voice`] — CELP voice frame
//! * [`acch`] — Layer-2 / RCH / SACCH and the ACCH inner fields
//! * [`l2_block`] — L2 multi-frame block dispatch + FACCH
//! * [`l2_text`] — text-format L2 data variants
//! * [`l2_binary`] — binary-format + SENDAI L2 data variants
//! * [`gps_emit`] — GPS sub-object emitters

mod acch;
mod gps_emit;
mod header;
mod l2_binary;
mod l2_block;
mod l2_text;
mod voice;

use crate::primitives::compare_sync_byte;
use crate::state::{DecoderState, MField, PscState};
use crate::tables::{SW_S2, SW_S6, SW_SS1};
use chrono::{Local, SecondsFormat};
use serde_json::{Map, Value, json};
use std::io::{self, Write};

/// Maximum sync-word symbol-error tolerance.
const MAX_SW_ERR: u32 = 3;

/// What [`Decoder`] writes per processed frame.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum OutputMode {
    /// One JSONL record per frame (default).
    #[default]
    Json,
    /// Only the 36-char hex `voice.celp` string (one per voice frame).
    /// Frames that did not produce a CELP payload (no_signal, header
    /// errors, blank/failed voice, non-VOICE M-field) emit nothing.
    CelpOnly,
}

/// High-level decoder façade.  Consume one 48-byte FDMA frame at a
/// time via [`Decoder::process_frame`] and write records (JSONL or
/// CELP, depending on [`OutputMode`]) to the underlying writer.
pub struct Decoder<W: Write> {
    out: W,
    state: DecoderState,
    psc: PscState,
    mode: OutputMode,
}

impl<W: Write> Decoder<W> {
    /// Create a decoder that emits JSONL.
    pub fn new(out: W) -> Self {
        Self::with_mode(out, OutputMode::default())
    }

    /// Create a decoder with an explicit output mode.
    pub fn with_mode(out: W, mode: OutputMode) -> Self {
        Decoder {
            out,
            state: DecoderState::new(),
            psc: PscState::default(),
            mode,
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    /// Process one 48-byte frame.  In [`OutputMode::Json`], `frame_num`
    /// is emitted as the `"frame"` field at the top of the JSONL
    /// record alongside a `"timestamp"` (RFC 3339, millisecond
    /// precision, local timezone) recording when decoding occurred;
    /// in [`OutputMode::CelpOnly`] both are unused (only the CELP hex
    /// string is written, when present).
    pub fn process_frame(&mut self, frame: &[u8; 48], frame_num: u64) -> io::Result<()> {
        let Decoder {
            out,
            state,
            psc,
            mode,
        } = self;

        let mut record = Map::new();
        record.insert("frame".to_string(), json!(frame_num));
        if *mode == OutputMode::Json {
            let ts = Local::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            record.insert("timestamp".to_string(), json!(ts));
        }

        if frame[17] == 0 {
            record.insert("type".to_string(), json!("no_signal"));
            *psc = PscState::default();
            state.reset_idle();
        } else if compare_sync_byte(&frame[0x17..], &SW_S6, MAX_SW_ERR) {
            proc_psc_branch(&mut record, state, psc, 0x1e56f, "S6", false, frame);
        } else if compare_sync_byte(&frame[0x17..], &SW_S2, MAX_SW_ERR) {
            proc_psc_branch(&mut record, state, psc, 0x9d236, "S2", true, frame);
        } else if compare_sync_byte(&frame[0x17..], &SW_SS1, MAX_SW_ERR) {
            proc_ss1_branch(&mut record, state, psc, frame);
        } else {
            record.insert("type".to_string(), json!("unknown_sync"));
            *psc = PscState::default();
            state.reset_idle();
        }

        finalize_super_frame(&mut record, state);

        match *mode {
            OutputMode::Json => {
                serde_json::to_writer(&mut *out, &Value::Object(record))?;
                out.write_all(b"\n")
            }
            OutputMode::CelpOnly => emit_celp_line(out, &record),
        }
    }
}

/// If `record["voice"]["celp"]` is present, write it followed by `\n`.
/// Otherwise write nothing.
fn emit_celp_line<W: Write>(out: &mut W, record: &Map<String, Value>) -> io::Result<()> {
    if let Some(Value::Object(voice)) = record.get("voice") {
        if let Some(Value::String(celp)) = voice.get("celp") {
            out.write_all(celp.as_bytes())?;
            out.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// Drive the post-frame super-frame counters and emit RCH/SACCH at the
/// canonical positions (2, 10, 18).
fn finalize_super_frame(record: &mut Map<String, Value>, state: &mut DecoderState) {
    if state.sacch_count == 2 {
        acch::proc_rch(record, state);
    }
    if state.sacch_count == 10 {
        acch::proc_sacch(record, state, 0);
    }
    if state.sacch_count == 18 {
        acch::proc_sacch(record, state, 1);
        state.sacch_count = 0;
    }
}

/// S2 / S6 frame branch.  Decodes the header, then runs the TCH /
/// SACCH / Layer-2 / voice pipeline; updates `psc` so the next
/// iteration can pair-decode consecutive frames.
fn proc_psc_branch(
    record: &mut Map<String, Value>,
    state: &mut DecoderState,
    psc: &mut PscState,
    ftype: u32,
    sync_label: &str,
    top_frame: bool,
    rbuf: &[u8; 48],
) {
    use crate::primitives::dewhite_psc_tch;

    record.insert("sync".to_string(), json!(sync_label));
    let hdr: [u8; 7] = rbuf[0x10..0x17].try_into().unwrap();
    let m = match header::proc_t61_frame(record, ftype, &hdr) {
        Some(m) => m,
        None => {
            psc.m = MField::Idle;
            return;
        }
    };

    let mut raw = [0u8; 35];
    dewhite_psc_tch(rbuf, &mut raw);
    header::proc_psc_sacch(record, state, &raw, top_frame);
    header::proc_psc_tch(record, &raw);

    let prev_tch = psc.tch;
    let mut m_eff = m;

    if matches!(psc.m, MField::Data | MField::Facch) {
        let f2: [u8; 32] = raw[..32].try_into().unwrap();
        acch::proc_layer2(record, state, &prev_tch, &f2, psc.m);
        if !matches!(m_eff, MField::Data | MField::Facch) {
            m_eff = MField::Idle; // suppress: wait for next frame
        }
    }
    if m_eff == MField::Voice && psc.m == MField::Voice {
        let f2: [u8; 32] = raw[..32].try_into().unwrap();
        voice::proc_voice(record, &prev_tch, &f2);
    }

    if matches!(m_eff, MField::Voice | MField::Data | MField::Facch) {
        psc.tch.copy_from_slice(&raw[..32]);
        psc.m = m_eff;
    } else {
        psc.m = MField::Idle;
        state.init_l2block();
    }
}

/// SS1 (centre-to-terminal sync acquired) branch: PICH paging.
fn proc_ss1_branch(
    record: &mut Map<String, Value>,
    state: &mut DecoderState,
    psc: &mut PscState,
    rbuf: &[u8; 48],
) {
    use crate::primitives::dewhite_pich;

    record.insert("sync".to_string(), json!("SS1"));
    let hdr: [u8; 7] = rbuf[0x10..0x17].try_into().unwrap();
    if header::proc_t61_frame(record, 0x2f94d06b, &hdr).is_some() {
        let mut raw = [0u8; 35];
        dewhite_pich(rbuf, &mut raw);
        header::proc_pich(record, &raw);
    }
    psc.m = MField::Idle;
    state.reset_idle();
}
