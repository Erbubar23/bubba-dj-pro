use super::onset::OnsetEnvelope;
use super::{snap_bpm, BeatGrid, BpmRange, GridSource, BEATS_PER_BAR};

/// Raw result of the DSP tracker (before choosing the bar anchor)
#[derive(Debug, Clone, Copy)]
pub struct DspEstimate {
    pub bpm: f64,
    /// Time of one beat (any beat) in seconds; the grid is `phase + k * 60 / bpm`
    pub phase_seconds: f64,
    /// First beat with a strong kick, in seconds
    pub first_beat_seconds: f64,
    /// Bar position guessed from accents: (beat index mod 4 relative to `phase_seconds`, strength 0..1)
    pub bar_phase: Option<(i64, f32)>,
    pub confidence: f32,
}

pub struct BeatTracker;

const PHASE_BINS: usize = 96;

impl BeatTracker {
    /// Detects BPM and anchors the first downbeat using only DSP (fast, < 0.3 s per track).
    pub fn analyze(mono_samples: &[f32], sample_rate: u32) -> BeatGrid {
        Self::analyze_range(mono_samples, sample_rate, BpmRange::default())
    }

    pub fn analyze_range(mono_samples: &[f32], sample_rate: u32, range: BpmRange) -> BeatGrid {
        if mono_samples.is_empty() || sample_rate == 0 {
            return BeatGrid::fallback();
        }
        let env = OnsetEnvelope::compute(mono_samples, sample_rate);
        match Self::estimate(&env, range, None) {
            Some(e) => Self::to_grid(&e),
            None => BeatGrid::fallback(),
        }
    }

    pub fn to_grid(e: &DspEstimate) -> BeatGrid {
        // Without a confident accent, the first strong kick is taken as beat 1
        let first_idx = ((e.first_beat_seconds - e.phase_seconds) / (60.0 / e.bpm)).round() as i64;
        let bar_phase = match e.bar_phase {
            Some((p, s)) if s >= 0.08 => p,
            _ => first_idx.rem_euclid(BEATS_PER_BAR),
        };
        BeatGrid::from_phase(e.bpm, e.phase_seconds, bar_phase, e.first_beat_seconds, e.confidence, GridSource::Dsp)
    }

    /// 1. Coarse tempo from autocorrelation of kick onsets inside `range` (or `tempo_hint`).
    /// 2. Fine tempo + phase from a comb (phase-folding) search over the WHOLE track,
    ///    which keeps the grid locked for minutes instead of drifting.
    /// 3. First beat = first strong kick on the grid (noisy / beatless intros are skipped).
    /// 4. Bar position from broadband accents (crashes, new elements land on the "1").
    pub fn estimate(env: &OnsetEnvelope, range: BpmRange, tempo_hint: Option<f64>) -> Option<DspEstimate> {
        if env.is_silent() {
            return None;
        }
        let rate = env.control_rate;
        let flux = &env.kick_flux;

        let (coarse_bpm, search) = match tempo_hint {
            Some(h) if h > 0.0 => (h, 0.006),
            _ => (Self::coarse_tempo(flux, rate, range)?, 0.015),
        };

        // ---- Fine tempo + phase via comb folding over the whole track ----
        let threshold = env.max_kick_flux * 0.05;
        let onsets: Vec<(f64, f32)> = flux
            .iter()
            .enumerate()
            .filter(|(_, &v)| v > threshold)
            .map(|(i, &v)| (i as f64, v))
            .collect();

        let mut best = (coarse_bpm, f32::MIN);
        let mut bpm = coarse_bpm * (1.0 - search);
        while bpm <= coarse_bpm * (1.0 + search) {
            let (_, score) = fold(&onsets, 60.0 * rate / bpm);
            if score > best.1 {
                best = (bpm, score);
            }
            bpm += 0.01;
        }

        let final_bpm = snap_bpm(best.0);
        let period = 60.0 * rate / final_bpm;
        let (phase, score) = fold(&onsets, period);
        // `phase` sits on the kick BODY here; it is moved onto the attack below
        let total_energy: f32 = onsets.iter().map(|o| o.1).sum();

        // ---- First beat: first strong kick on the grid ----
        let window = (period / 8.0).max(1.0) as usize;
        let mut peaks = Vec::new();
        let mut t = phase;
        while t < flux.len() as f64 {
            peaks.push(OnsetEnvelope::peak_near(flux, t, window));
            t += period;
        }
        let mut sorted = peaks.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let typical = sorted.get(sorted.len() * 3 / 4).copied().unwrap_or(env.max_kick_flux);
        let first_beat_index = peaks.iter().position(|&p| p >= typical * 0.35).unwrap_or(0);

        // Grid lines go on the attack of the kick, not on its body
        let phase = align_to_attack(env, phase, period);

        // ---- Bar position from broadband accents ----
        // Mean (not sum) per position: a short clip has more beats on some positions
        let mut acc = [0.0f64; BEATS_PER_BAR as usize];
        let mut counts = [0usize; BEATS_PER_BAR as usize];
        for (k, _) in peaks.iter().enumerate().skip(first_beat_index) {
            let center = phase + k as f64 * period;
            acc[k % BEATS_PER_BAR as usize] += OnsetEnvelope::peak_near(&env.full_flux, center, window) as f64;
            counts[k % BEATS_PER_BAR as usize] += 1;
        }
        for (a, &n) in acc.iter_mut().zip(&counts) {
            *a /= n.max(1) as f64;
        }
        let sum: f64 = acc.iter().sum();
        let bar_phase = if sum > 0.0 {
            let (p, &m) = acc.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            let mean = sum / acc.len() as f64;
            Some((p as i64, ((m - mean) / mean) as f32))
        } else {
            None
        };

        let confidence = if total_energy > 0.0 {
            (score / total_energy * PHASE_BINS as f32 / 12.0).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Some(DspEstimate {
            bpm: final_bpm,
            phase_seconds: phase / rate,
            first_beat_seconds: (phase + first_beat_index as f64 * period) / rate,
            bar_phase,
            confidence,
        })
    }

    /// Autocorrelation of the first 90 s of kick onsets, best lag inside `range`
    fn coarse_tempo(flux: &[f32], rate: f64, range: BpmRange) -> Option<f64> {
        let min_lag = (rate * 60.0 / range.max).floor().max(2.0) as usize;
        let max_lag = (rate * 60.0 / range.min).ceil() as usize;
        let analysis_len = ((90.0 * rate) as usize).min(flux.len());
        let slice = &flux[..analysis_len];
        if slice.len() <= max_lag + 2 {
            return None;
        }

        let mut corr = vec![0.0f32; max_lag + 2];
        for lag in (min_lag - 1)..=(max_lag + 1) {
            let count = slice.len() - lag;
            corr[lag] = (0..count).map(|i| slice[i] * slice[i + lag]).sum();
        }

        let mut best_lag = min_lag;
        for lag in min_lag..=max_lag {
            if corr[lag] > corr[best_lag] {
                best_lag = lag;
            }
        }

        // Parabolic interpolation for sub-sample period
        let (c0, c1, c2) = (corr[best_lag - 1] as f64, corr[best_lag] as f64, corr[best_lag + 1] as f64);
        let denom = c0 - 2.0 * c1 + c2;
        let offset = if denom.abs() > 1e-12 { (0.5 * (c0 - c2) / denom).clamp(-0.5, 0.5) } else { 0.0 };
        Some(range.fold(60.0 * rate / (best_lag as f64 + offset)))
    }
}

/// The kick-band flux peaks on the BODY of long / distorted kicks (hard techno: ~100 ms
/// after the hit). Slides the phase back onto the broadband attack (the click), searching
/// only before the body so off-beat hats (half a beat away) are never picked.
fn align_to_attack(env: &OnsetEnvelope, phase: f64, period: f64) -> f64 {
    const BACK_SECONDS: f64 = 0.15;
    const FORWARD_SECONDS: f64 = 0.02;
    let rate = env.control_rate;
    let flux = &env.full_flux;
    let beats = (flux.len() as f64 / period) as usize;
    let score = |offset: f64| -> f32 {
        (0..beats)
            .map(|k| OnsetEnvelope::peak_near(flux, phase + offset + k as f64 * period, 2))
            .sum()
    };
    let at_body = score(0.0);
    let (mut best_offset, mut best) = (0.0, at_body);
    let mut offset = -BACK_SECONDS * rate;
    while offset <= FORWARD_SECONDS * rate {
        let s = score(offset);
        if s > best {
            best = s;
            best_offset = offset;
        }
        offset += 1.0;
    }
    // Only move when the attack is clearly there (pure sine kicks have no click)
    if best > at_body * 1.5 {
        phase + best_offset
    } else {
        phase
    }
}

/// Folds all onsets modulo `period` into a phase histogram.
/// Returns (phase_in_frames, peak_score).
fn fold(onsets: &[(f64, f32)], period: f64) -> (f64, f32) {
    let mut bins = [0.0f32; PHASE_BINS];
    for &(t, v) in onsets {
        let ph = (t / period).fract();
        let idx = ((ph * PHASE_BINS as f64) as usize).min(PHASE_BINS - 1);
        bins[idx] += v;
    }

    // Smooth over 3 neighbouring bins (circular)
    let mut best_idx = 0;
    let mut best_score = f32::MIN;
    for i in 0..PHASE_BINS {
        let s = bins[(i + PHASE_BINS - 1) % PHASE_BINS] * 0.5 + bins[i] + bins[(i + 1) % PHASE_BINS] * 0.5;
        if s > best_score {
            best_score = s;
            best_idx = i;
        }
    }

    // Centre of mass around the winning bin for sub-bin precision
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    for d in -2i32..=2 {
        let idx = (best_idx as i32 + d).rem_euclid(PHASE_BINS as i32) as usize;
        num += d as f64 * bins[idx] as f64;
        den += bins[idx] as f64;
    }
    let refined = best_idx as f64 + 0.5 + if den > 0.0 { num / den } else { 0.0 };
    let phase = (refined / PHASE_BINS as f64).rem_euclid(1.0) * period;

    (phase, best_score)
}
