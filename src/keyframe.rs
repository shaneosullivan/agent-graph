//! Keyframes in a log sent to the site: every `EVERY` events, the reducer's
//! whole state (`reducer::Replay::keyframe`), so the site can keep only a
//! log's last two keyframes' worth and still show where everything stands.
//!
//! A log to send is cut (`Trimmer`) to start at its last keyframe but one,
//! and has its last: so it holds between `EVERY` and twice that many
//! events. It's read a few lines at a time, keeping only where each event is
//! and where it sorts, then replayed in order, each line read again as it's
//! applied, so a big log takes little more memory than its text; and a
//! page can show how far along it is (site/lib/trim-worker.ts).

use std::ops::Range;
use std::time::SystemTime;

use crate::event::Envelope;
use crate::reducer::{self, KEYFRAME_PART, KEYFRAME_PARTS, Replay};

/// Events between keyframes.
pub const EVERY: usize = 1000;

/// Where an event is in a log's text, and where it sorts.
#[derive(Debug, Clone)]
pub struct Line {
    pub key: (SystemTime, String),
    pub at: Range<usize>,
}

/// A log cut to send (see `Trimmer`).
pub struct Trimmed {
    /// Its lines: the keyframe it starts at, if any, its events (as they
    /// were written), and its last keyframe, if it has room for one.
    pub text: Vec<u8>,
    /// Where each keyframe starts in `text`.
    pub keyframes: Vec<usize>,
    /// How many events it sends, of how many there are.
    pub events: usize,
    pub of: usize,
    /// The log's events, in order, in the text it was cut from.
    pub lines: Vec<Line>,
    /// The index of the first event it sends, and the state before it.
    pub from: usize,
    pub at_from: Replay,
    /// The index of the event after its last keyframe, and the state there.
    pub last: usize,
    pub replay: Replay,
}

enum Phase {
    /// Reading lines, from this byte.
    Reading(usize),
    /// Applying events in order: the next one.
    Replaying(usize),
}

/// Cuts a log to send, a step at a time. Its text can start from a keyframe
/// (a log from the site, say: its leading keyframe lines); a keyframe later
/// in it stands for events already there, and is left out, as are lines
/// that aren't events.
pub struct Trimmer {
    text: Vec<u8>,
    every: usize,
    phase: Phase,
    /// The leading keyframe's parts, while there's been nothing else.
    parts: Vec<Envelope>,
    leading: bool,
    lines: Vec<Line>,
    replay: Replay,
    at_from: Replay,
    from: usize,
    last: usize,
    start: Vec<Envelope>,
    keyframe: Vec<Envelope>,
    /// Replaying every event, for its state (`replay_all`), not cutting.
    whole: bool,
}

/// The state after every event in `text`, in order (see `Trimmer`), and
/// the last of them; none if it has none.
pub fn replay_all(text: Vec<u8>) -> Option<(Replay, Envelope)> {
    let mut trimmer = Trimmer::new(text, 1);
    trimmer.whole = true;
    while !trimmer.step(usize::MAX) {}
    let last = trimmer.event(trimmer.lines.len().checked_sub(1)?);
    Some((trimmer.replay, last))
}

impl Trimmer {
    pub fn new(text: Vec<u8>, every: usize) -> Trimmer {
        Trimmer {
            text,
            every: every.max(1),
            phase: Phase::Reading(0),
            parts: Vec::new(),
            leading: true,
            lines: Vec::new(),
            replay: Replay::default(),
            at_from: Replay::default(),
            from: 0,
            last: 0,
            start: Vec::new(),
            keyframe: Vec::new(),
            whole: false,
        }
    }

    /// Does up to `lines` lines' work. Returns whether it's done.
    pub fn step(&mut self, lines: usize) -> bool {
        for _ in 0..lines.max(1) {
            match self.phase {
                Phase::Reading(at) if at >= self.text.len() => self.start_replaying(),
                Phase::Reading(at) => self.read(at),
                Phase::Replaying(i) if i >= self.last => return true,
                Phase::Replaying(i) => self.apply(i),
            }
        }
        matches!(self.phase, Phase::Replaying(i) if i >= self.last)
    }

    /// How far along it is, from 0 to 1: reading, then replaying, each half.
    pub fn progress(&self) -> f64 {
        match self.phase {
            Phase::Reading(at) => at as f64 / self.text.len().max(1) as f64 / 2.0,
            Phase::Replaying(i) => 0.5 + i as f64 / self.last.max(1) as f64 / 2.0,
        }
    }

    fn read(&mut self, at: usize) {
        let end = self.text[at..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.text.len(), |i| at + i + 1);
        self.phase = Phase::Reading(end);
        let Ok(event) = serde_json::from_slice::<Envelope>(&self.text[at..end]) else {
            return;
        };
        if reducer::is_keyframe(&event) {
            if self.leading && self.parts.len() < KEYFRAME_PARTS {
                self.parts.push(event);
            }
            return;
        }
        self.leading = false;
        self.lines.push(Line {
            key: reducer::sort_key(&event),
            at: at..end,
        });
    }

    fn start_replaying(&mut self) {
        self.lines.sort_by(|a, b| a.key.cmp(&b.key));
        let base = reducer::merge_keyframe(&std::mem::take(&mut self.parts));
        self.replay = Replay::new(base.as_ref());
        self.at_from = self.replay.clone();
        self.last = self.lines.len() / self.every * self.every;
        self.from = self.last.saturating_sub(self.every);
        if self.whole {
            (self.from, self.last) = (0, self.lines.len());
        }
        // With room for fewer than two, it starts at its start: its base.
        if let (0, Some(base), false) = (self.from, &base, self.whole) {
            self.start = self.replay.keyframe(base, KEYFRAME_PART);
        }
        self.phase = Phase::Replaying(0);
    }

    fn apply(&mut self, i: usize) {
        let event = self.event(i);
        self.replay.apply(&event);
        if self.whole {
            self.phase = Phase::Replaying(i + 1);
            return;
        }
        if i + 1 == self.from {
            self.start = self.replay.keyframe(&event, KEYFRAME_PART);
            self.at_from = self.replay.clone();
        }
        if i + 1 == self.last {
            self.keyframe = self.replay.keyframe(&event, KEYFRAME_PART);
        }
        self.phase = Phase::Replaying(i + 1);
    }

    fn event(&self, i: usize) -> Envelope {
        serde_json::from_slice(&self.text[self.lines[i].at.clone()]).expect("read before")
    }

    /// The log to send (doing whatever work is left), and its text.
    pub fn finish(mut self) -> (Trimmed, Vec<u8>) {
        while !self.step(usize::MAX) {}
        let mut text = Vec::new();
        let mut keyframes = Vec::new();
        let add_keyframe = |text: &mut Vec<u8>, keyframes: &mut Vec<usize>, parts: &[Envelope]| {
            if !parts.is_empty() {
                keyframes.push(text.len());
            }
            for part in parts {
                text.extend(serde_json::to_vec(part).expect("serializable"));
                text.push(b'\n');
            }
        };
        let add_lines = |text: &mut Vec<u8>, lines: &[Line]| {
            for line in lines {
                let bytes = &self.text[line.at.clone()];
                text.extend_from_slice(bytes);
                if !bytes.ends_with(b"\n") {
                    text.push(b'\n');
                }
            }
        };
        add_keyframe(&mut text, &mut keyframes, &self.start);
        add_lines(&mut text, &self.lines[self.from..self.last]);
        add_keyframe(&mut text, &mut keyframes, &self.keyframe);
        add_lines(&mut text, &self.lines[self.last..]);
        let of = self.lines.len();
        (
            Trimmed {
                text,
                keyframes,
                events: of - self.from,
                of,
                lines: self.lines,
                from: self.from,
                at_from: self.at_from,
                last: self.last,
                replay: self.replay,
            },
            self.text,
        )
    }
}

/// `text` cut to send: see `Trimmer`.
pub fn trim(text: &[u8], every: usize) -> Trimmed {
    Trimmer::new(text.to_vec(), every).finish().0
}
