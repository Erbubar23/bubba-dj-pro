//! Beat grid: data model + detectors.
//!
//! - `onset`  : kick / broadband onset envelopes shared by every detector
//! - `dsp`    : fast signal-processing tracker (instant, used while loading)
//! - `fit`    : fits a constant-tempo grid to a list of beat timestamps
//! - `neural` : Beat This! (ISMIR 2024) beat + downbeat model (feature `neural`)
//! - `refine` : combines neural + DSP into the final grid
//!
//! The grid is constant-tempo (like rekordbox / Serato default grids): one BPM,
//! one anchor on beat 1 of a bar, 4 beats per bar.

pub mod onset;
pub mod dsp;
pub mod fit;
pub mod refine;
#[cfg(feature = "neural")]
pub mod neural;

pub use dsp::BeatTracker;

pub const BEATS_PER_BAR: i64 = 4;

/// Which detector produced the grid (shown in the UI, used by the analysis cache)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GridSource {
    #[default]
    Dsp,
    Neural,
    /// Edited by the user (never overwritten by re-analysis)
    Manual,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BeatGrid {
    /// Track tempo in Beats Per Minute (e.g. 126.0)
    pub bpm: f32,
    /// Timestamp in seconds of a downbeat (beat 1 of a bar). Every 4th beat from here is a "1".
    /// It is the bar that contains the first beat with sound, so it can be slightly before it.
    pub first_downbeat_seconds: f64,
    /// Distance between beats in seconds
    pub beat_interval_seconds: f64,
    /// Confidence score (0.0 .. 1.0) of the grid
    pub confidence: f32,
    /// First beat with sound (default CUE point, like rekordbox's auto cue)
    #[serde(default)]
    pub first_beat_seconds: Option<f64>,
    #[serde(default)]
    pub source: GridSource,
}

impl BeatGrid {
    /// Constant grid with the downbeat at `first_downbeat_seconds`
    pub fn constant(bpm: f32, first_downbeat_seconds: f64) -> Self {
        BeatGrid {
            bpm,
            first_downbeat_seconds,
            beat_interval_seconds: 60.0 / bpm.max(1.0) as f64,
            confidence: 1.0,
            first_beat_seconds: None,
            source: GridSource::Dsp,
        }
    }

    pub(crate) fn fallback() -> Self {
        BeatGrid { confidence: 0.0, ..Self::constant(120.0, 0.0) }
    }

    /// Default CUE position in seconds
    pub fn cue_seconds(&self) -> f64 {
        self.first_beat_seconds.unwrap_or(self.first_downbeat_seconds).max(0.0)
    }

    /// Continuous beat count at `seconds` (0.0 = the anchor downbeat)
    pub fn beat_at(&self, seconds: f64) -> f64 {
        (seconds - self.first_downbeat_seconds) / self.beat_interval_seconds
    }

    pub fn time_of_beat(&self, beat: f64) -> f64 {
        self.first_downbeat_seconds + beat * self.beat_interval_seconds
    }

    /// Builds a grid from tempo + phase, choosing the bar anchor so beat `bar_phase`
    /// (mod 4) lands on the "1", placed at or before the first beat with sound.
    pub(crate) fn from_phase(bpm: f64, beat_phase_seconds: f64, bar_phase: i64, first_beat_seconds: f64, confidence: f32, source: GridSource) -> Self {
        let interval = 60.0 / bpm;
        // Index (relative to beat_phase_seconds) of the first beat with sound
        let first_idx = ((first_beat_seconds - beat_phase_seconds) / interval).round() as i64;
        // Latest downbeat index <= first_idx
        let down_idx = first_idx - (first_idx - bar_phase).rem_euclid(BEATS_PER_BAR);
        let mut anchor = beat_phase_seconds + down_idx as f64 * interval;
        if anchor < -1e-3 {
            anchor += BEATS_PER_BAR as f64 * interval;
        }
        BeatGrid {
            bpm: ((bpm * 100.0).round() / 100.0) as f32,
            first_downbeat_seconds: anchor,
            beat_interval_seconds: interval,
            confidence: confidence.clamp(0.0, 1.0),
            first_beat_seconds: Some((beat_phase_seconds + first_idx as f64 * interval).max(0.0)),
            source,
        }
    }
}

/// DJ music is almost always on an integer (or .5) tempo: snap when very close
pub(crate) fn snap_bpm(bpm: f64) -> f64 {
    for snap in [bpm.round(), (bpm * 2.0).round() / 2.0] {
        if (bpm - snap).abs() < 0.04 {
            return snap;
        }
    }
    bpm
}

/// Tempo search range. Tempos outside are folded by octaves (×2 / ÷2) into it,
/// like the "BPM range" analysis option of rekordbox.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct BpmRange {
    pub min: f64,
    pub max: f64,
}

impl Default for BpmRange {
    fn default() -> Self {
        BpmRange { min: 85.0, max: 175.0 }
    }
}

impl BpmRange {
    pub fn fold(&self, mut bpm: f64) -> f64 {
        if !(bpm > 0.0) {
            return bpm;
        }
        while bpm < self.min && bpm * 2.0 <= self.max * 1.02 {
            bpm *= 2.0;
        }
        while bpm > self.max && bpm / 2.0 >= self.min * 0.98 {
            bpm /= 2.0;
        }
        bpm
    }
}
