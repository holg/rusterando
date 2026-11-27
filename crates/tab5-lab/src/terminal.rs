//! The terminal's state, whatever draws it: output lines with their tone,
//! the status line, the prompt and the input line with its cursor. tab5-lab
//! draws it full-screen (console.rs); the kitchen firmware (crates/kds-tab5)
//! includes this file and shows it on its Slint "Terminal" page.
//!
//! Lines are kept whole — each renderer wraps them to its own width — and
//! every line pushed (and every clear) is also kept for the phone
//! ([`Terminal::take_mirror`], remote.rs).

use std::collections::VecDeque;

/// Output lines kept (the screens show the newest that fit).
const KEEP_LINES: usize = 200;

/// What an output line is, by colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Dim,
    Echo,
    Ok,
    Rx,
    Tx,
    Warn,
    Err,
}

impl Tone {
    /// 0 normal … 7 error: the kitchen UI's TermLine tone, the phone's
    /// letters n d e o r t w x in this order.
    pub fn index(self) -> i32 {
        self as i32
    }
}

/// What the screen did, for the phone (remote.rs): each output line as
/// pushed, and clearing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mirror {
    Line(Tone, String),
    Clear,
}

pub struct Terminal {
    lines: VecDeque<(Tone, String)>,
    /// Since the last [`Terminal::take_mirror`].
    mirror: Vec<Mirror>,
    pub status: String,
    pub prompt: String,
    pub input: String,
    /// Cursor position in `input`, in chars.
    pub cursor: usize,
    dirty: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            mirror: Vec::new(),
            status: String::new(),
            prompt: String::new(),
            input: String::new(),
            cursor: 0,
            dirty: true,
        }
    }
}

impl Terminal {
    /// Append output; `\n` starts a new line.
    pub fn push(&mut self, tone: Tone, text: &str) {
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r').replace('\t', " ");
            self.mirror.push(Mirror::Line(tone, line.clone()));
            self.lines.push_back((tone, line));
        }
        while self.lines.len() > KEEP_LINES {
            self.lines.pop_front();
        }
        self.dirty = true;
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.mirror.push(Mirror::Clear);
        self.dirty = true;
    }

    /// The lines pushed (and clears) since the last call.
    pub fn take_mirror(&mut self) -> Vec<Mirror> {
        std::mem::take(&mut self.mirror)
    }

    /// Output lines, oldest first.
    pub fn lines(&self) -> &VecDeque<(Tone, String)> {
        &self.lines
    }

    /// The input line split at the cursor (the kitchen UI draws the cursor
    /// between the two).
    #[allow(dead_code)]
    pub fn input_split(&self) -> (String, String) {
        let at = self
            .input
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        (self.input[..at].to_string(), self.input[at..].to_string())
    }

    /// Something changed: redraw.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Drawn: clean until the next change.
    pub fn drawn(&mut self) {
        self.dirty = false;
    }
}
