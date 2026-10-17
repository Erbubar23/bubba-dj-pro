use super::dsp::{BeatTracker, DspEstimate};
use super::fit::GridFit;
use super::onset::OnsetEnvelope;
use super::{snap_bpm, BeatGrid, BpmRange, GridSource, BEATS_PER_BAR};

/// A neural fit is trusted only when most beats sit on one constant grid
const MIN_COHERENCE: f32 = 0.6;
const MIN_INLIERS: f32 = 0.6;
/// Downbeat votes needed to take the bar position from the model
const MIN_DOWNBEAT_SHARE: f32 = 0.4;
/// Max disagreement between the DSP (kick) phase and the neural phase
const MAX_PHASE_GAP_SECONDS: f64 = 0.04;

/// Final grid from the neural fit (tempo octave + bar position) and the DSP tracker
/// (exact tempo over the whole track + kick-aligned phase). Falls back to DSP alone
/// when the neural fit is missing or unreliable.
pub fn combine(env: &OnsetEnvelope, fit: Option<&GridFit>, range: BpmRange) -> Option<BeatGrid> {
    let fit = match fit {
        Some(f) if f.coherence >= MIN_COHERENCE && f.inliers >= MIN_INLIERS => f,
        _ => return BeatTracker::estimate(env, range, None).map(|e| BeatTracker::to_grid(&e)),
    };

    let seeded = BeatTracker::estimate(env, range, Some(fit.bpm));
    let (bpm, phase) = match seeded {
        Some(d) if agrees(&d, fit) => (d.bpm, d.phase_seconds),
        _ => (snap_bpm(fit.bpm), fit.phase_seconds),
    };
    let period = 60.0 / bpm;
    let first_beat = seeded.map(|d| d.first_beat_seconds).unwrap_or(phase);
    let confidence = (fit.coherence * fit.inliers).sqrt();

    let bar_phase = match (fit.downbeat, seeded) {
        (Some((t, share)), _) if share >= MIN_DOWNBEAT_SHARE => ((t - phase) / period).round() as i64,
        // Model unsure about bars: keep its tempo/phase, take the bar from DSP accents
        (_, Some(d)) => {
            let g = BeatTracker::to_grid(&DspEstimate { bpm, phase_seconds: phase, first_beat_seconds: first_beat, ..d });
            return Some(BeatGrid { source: GridSource::Neural, confidence, ..g });
        }
        _ => ((first_beat - phase) / period).round() as i64,
    };

    Some(BeatGrid::from_phase(bpm, phase, bar_phase.rem_euclid(BEATS_PER_BAR), first_beat, confidence, GridSource::Neural))
}

/// Same tempo and same beat phase (compared in the middle of the analyzed excerpts)
fn agrees(d: &DspEstimate, fit: &GridFit) -> bool {
    if (d.bpm - fit.bpm).abs() > 0.15 {
        return false;
    }
    let period = 60.0 / fit.bpm;
    let at_ref = |phase: f64, bpm: f64| {
        let p = 60.0 / bpm;
        phase + ((fit.ref_time - phase) / p).round() * p
    };
    let gap = (at_ref(d.phase_seconds, d.bpm) - at_ref(fit.phase_seconds, fit.bpm)) / period;
    (gap - gap.round()).abs() * period <= MAX_PHASE_GAP_SECONDS
}
