//! Decoder state types: M-field, per-PSC carry-over, and the
//! multi-frame Layer-2 / SACCH block reassembly buffers.

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum MField {
    #[default]
    Idle = 0,
    Voice = 1,
    Raw = 2,
    Data = 3,
    Facch = 4,
    Free = 5,
    Busy = 6,
    Undef = 7,
}

impl MField {
    /// Decode the 3-bit RICH `r[5..7]` index into an [`MField`].
    pub fn from_idx(idx: u8) -> Self {
        match idx & 7 {
            0 => MField::Idle,
            1 => MField::Voice,
            2 => MField::Raw,
            3 => MField::Data,
            4 => MField::Facch,
            5 => MField::Free,
            6 => MField::Busy,
            _ => MField::Undef,
        }
    }

    /// Name of the M-field, as emitted in the `"mfield"` JSON field.
    pub fn name(self) -> &'static str {
        match self {
            MField::Idle => "IDLE",
            MField::Voice => "VOICE",
            MField::Raw => "RAW",
            MField::Data => "DATA",
            MField::Facch => "FACCH",
            MField::Free => "FREE",
            MField::Busy => "BUSY",
            MField::Undef => "UNDEF",
        }
    }
}

/// Carries previous-frame TCH state across PSC iterations.  Used to
/// pair-up two consecutive frames for Layer-2 / voice CELP decoding.
#[derive(Default)]
pub struct PscState {
    pub m: MField,
    pub tch: [u8; 32],
}

/// Multi-frame state: SACCH/RCH gathering for the super-frame plus
/// Layer-2 / SACCH multi-frame block reassembly.
pub struct DecoderState {
    /// Symbol counter within the current 18-step super-frame.
    pub sacch_count: usize,
    pub sacch_buf: [[u8; 3]; 18],
    pub rch: [u8; 5],
    pub sacch: [[u8; 20]; 2],

    pub l2blocks: [u8; 12 * 64],
    pub l2block_count: usize,
    pub l2block_last_len: usize,

    pub sacch_blocks: [u8; 6 * 64],
    pub sacch_block_count: usize,
    pub sacch_block_last_len: usize,
}

impl Default for DecoderState {
    fn default() -> Self {
        DecoderState {
            sacch_count: 0,
            sacch_buf: [[0; 3]; 18],
            rch: [0; 5],
            sacch: [[0; 20]; 2],
            l2blocks: [0; 12 * 64],
            l2block_count: 0,
            l2block_last_len: 0,
            sacch_blocks: [0; 6 * 64],
            sacch_block_count: 0,
            sacch_block_last_len: 0,
        }
    }
}

impl DecoderState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init_l2block(&mut self) {
        self.l2block_count = 0;
        self.l2block_last_len = 0;
    }

    pub fn init_sacch_block(&mut self) {
        self.sacch_block_count = 0;
        self.sacch_block_last_len = 0;
    }

    pub fn alloc_l2block(&mut self, count: usize) {
        self.l2block_count = count;
        self.l2blocks.fill(0);
    }

    pub fn alloc_sacch_block(&mut self, count: usize) {
        self.sacch_block_count = count;
    }

    pub fn assemble_l2block(&mut self, f_pos: usize, src: &[u8]) {
        if self.l2block_count == 0 {
            return;
        }
        let off = 12 * (self.l2block_count - f_pos - 1);
        self.l2blocks[off..off + src.len()].copy_from_slice(src);
        self.l2block_last_len = src.len();
    }

    pub fn assemble_sacch_block(&mut self, f_pos: usize, src: &[u8]) {
        if self.sacch_block_count == 0 {
            return;
        }
        let off = 6 * (self.sacch_block_count - f_pos - 1);
        self.sacch_blocks[off..off + src.len()].copy_from_slice(src);
        self.sacch_block_last_len = src.len();
    }

    /// Reset to "channel idle" (used on no_signal / SS1 / unknown frames).
    pub fn reset_idle(&mut self) {
        self.sacch_count = 0;
        self.init_l2block();
        self.init_sacch_block();
    }

    /// Concatenated length of the assembled L2 block.
    pub fn l2block_total_len(&self) -> usize {
        if self.l2block_count == 0 {
            0
        } else {
            (self.l2block_count - 1) * 12 + self.l2block_last_len
        }
    }

    /// Concatenated length of the assembled SACCH block.
    pub fn sacch_block_total_len(&self) -> usize {
        if self.sacch_block_count == 0 {
            0
        } else {
            (self.sacch_block_count - 1) * 6 + self.sacch_block_last_len
        }
    }
}
