use super::dsp::{BeatTracker, DspEstimate};
use super::fit::GridFit;
use super::onset::OnsetEnvelope;
use super::{snap_bpm, BeatGrid, BpmRange, GridSource, BEATS_PER_BAR};

/// A neural fit is trusted only when most beats sit on one constant grid
const MIN_COHERENCE: f32 = 0.6;
const MIN_INLIERS: f32 = 0.6;
/// Downbeat votes needed to take the bar position from the model
const MIN_DOWNBEAT_SHARE: f32 = 0.4;
/// DSP and model beats this far apart (in beats) mean one of them locked onto the
/// off-beats (hats, rumble): the one on the stronger attack wins
const OFF_BEAT_GAP: f64 = 0.35;
/// Same tempo for DSP and model (relative): the fit on short material is less precise
const MAX_TEMPO_GAP: f64 = 0.003;

/// Final grid from the neural fit (tempo octave, beat phase, bar position) and the DSP
/// tracker (exact tempo over the whole track, first kick). Falls back to DSP alone when
/// the neural fit is missing or unreliable.
pub fn combine(env: &OnsetEnvelope, fit: Option<&GridFit>, range: BpmRange) -> Option<BeatGrid> {
    let fit = match fit {
        Some(f) if f.coherence >= MIN_COHERENCE && f.inliers >= MIN_INLIERS => f,
        _ => return BeatTracker::estimate(env, range, None).map(|e| BeatTracker::to_grid(&e)),
    };

    // DSP seeded with the model's tempo: right octave, exact tempo over the whole track.
    // The model's phase (trained on annotated beats) wins unless it sits on the off-beats.
    let seeded = BeatTracker::estimate(env, range, Some(fit.bpm))
        .filter(|d| (d.bpm - fit.bpm).abs() <= fit.bpm * MAX_TEMPO_GAP);
    let (bpm, phase, model_phase_ok) = match seeded {
        Some(d) if phase_gap(&d, fit) > OFF_BEAT_GAP => {
            let frames = |s: f64| s * env.control_rate;
            let period = frames(60.0 / d.bpm);
            let model = model_beat_near_ref(fit);
            if env.attack_strength(frames(model), period) > env.attack_strength(frames(d.phase_seconds), period) {
                (d.bpm, model, true)
            } else {
                (d.bpm, d.phase_seconds, false)
            }
        }
        Some(d) => (d.bpm, model_beat_near_ref(fit), true),
        None => (snap_bpm(fit.bpm), fit.phase_seconds, true),
    };
    let period = 60.0 / bpm;
    let first_beat = seeded.map(|d| d.first_beat_seconds).unwrap_or(phase);
    let mut confidence = (fit.coherence * fit.inliers).sqrt();
    if !model_phase_ok {
        // The model locked onto something else (off-beat hats...): its downbeats are not on our beats
        confidence *= 0.7;
    }

    let downbeat = fit.downbeat.filter(|&(_, share)| model_phase_ok && share >= MIN_DOWNBEAT_SHARE);
    let bar_phase = match (downbeat, seeded) {
        (Some((t, _)), _) => ((t - phase) / period).round() as i64,
        // Model unsure about bars: keep the tempo/phase, take the bar from DSP accents
        (None, Some(d)) => {
            let g = BeatTracker::to_grid(&DspEstimate { bpm, phase_seconds: phase, first_beat_seconds: first_beat, ..d });
            return Some(BeatGrid { source: GridSource::Neural, confidence, ..g });
        }
        (None, None) => ((first_beat - phase) / period).round() as i64,
    };

    Some(BeatGrid::from_phase(bpm, phase, bar_phase.rem_euclid(BEATS_PER_BAR), first_beat, confidence, GridSource::Neural))
}

/// A model beat in the middle of the analyzed excerpts: a valid phase for any tempo
/// close to the model's (extrapolating from t = 0 would amplify a tiny tempo difference)
fn model_beat_near_ref(fit: &GridFit) -> f64 {
    beat_near(fit.phase_seconds, fit.bpm, fit.ref_time)
}

fn beat_near(phase: f64, bpm: f64, t: f64) -> f64 {
    let p = 60.0 / bpm;
    phase + ((t - phase) / p).round() * p
}

/// Distance between the DSP and model beat phases, in beats (0 .. 0.5), measured in
/// the middle of the analyzed excerpts
fn phase_gap(d: &DspEstimate, fit: &GridFit) -> f64 {
    let gap = (beat_near(d.phase_seconds, d.bpm, fit.ref_time) - model_beat_near_ref(fit)) / (60.0 / fit.bpm);
    (gap - gap.round()).abs()
}
