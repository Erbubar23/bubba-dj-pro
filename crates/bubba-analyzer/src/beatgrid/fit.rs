use std::f64::consts::TAU;

use super::{BpmRange, BEATS_PER_BAR};

/// Constant-tempo grid fitted to a list of beat timestamps
#[derive(Debug, Clone, Copy)]
pub struct GridFit {
    pub bpm: f64,
    /// Time of one beat; the grid is `phase + k * 60 / bpm`
    pub phase_seconds: f64,
    /// Phase coherence of the beats on the grid (0 = random, 1 = perfect)
    pub coherence: f32,
    /// Share of beats within ±35 ms of the grid
    pub inliers: f32,
    /// A downbeat time that agrees with the winning bar position, and the share of downbeats that agree
    pub downbeat: Option<(f64, f32)>,
    /// Middle of the analyzed material (phases are compared here)
    pub ref_time: f64,
}

const INLIER_SECONDS: f64 = 0.035;

/// Fits tempo + phase by maximizing the circular coherence of all beats.
/// Robust to the 20 ms frame quantization of neural beat trackers (a plain
/// median of intervals is biased by it) and to missing / extra beats.
pub fn fit_constant(beats: &[f64], downbeats: &[f64], range: BpmRange) -> Option<GridFit> {
    if beats.len() < 16 {
        return None;
    }
    let mut ibis: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).filter(|&d| d > 0.2 && d < 1.5).collect();
    if ibis.len() < 8 {
        return None;
    }
    ibis.sort_by(f64::total_cmp);
    let guess = range.fold(60.0 / ibis[ibis.len() / 2]);

    let mut best = coherence(beats, guess);
    let mut bpm = guess * 0.97;
    while bpm <= guess * 1.03 {
        let c = coherence(beats, bpm);
        if c.0 > best.0 {
            best = c;
        }
        bpm += 0.002;
    }
    let coarse = best.2;
    let mut bpm = coarse - 0.004;
    while bpm <= coarse + 0.004 {
        let c = coherence(beats, bpm);
        if c.0 > best.0 {
            best = c;
        }
        bpm += 0.0002;
    }

    let (r, phase, bpm) = best;
    let period = 60.0 / bpm;
    let residual = |t: f64| ((t - phase) / period - ((t - phase) / period).round()) * period;
    let inliers = beats.iter().filter(|&&t| residual(t).abs() < INLIER_SECONDS).count() as f32 / beats.len() as f32;

    // Bar position: vote with every downbeat that lands on the grid
    let mut votes = [0usize; BEATS_PER_BAR as usize];
    let mut example = [None; BEATS_PER_BAR as usize];
    let mut counted = 0;
    for &d in downbeats {
        if residual(d).abs() >= INLIER_SECONDS * 2.0 {
            continue;
        }
        let p = (((d - phase) / period).round() as i64).rem_euclid(BEATS_PER_BAR) as usize;
        votes[p] += 1;
        example[p].get_or_insert(d);
        counted += 1;
    }
    let downbeat = if counted >= 4 {
        let (p, &n) = votes.iter().enumerate().max_by_key(|v| *v.1).unwrap();
        example[p].map(|d| (d, n as f32 / counted as f32))
    } else {
        None
    };

    Some(GridFit {
        bpm,
        phase_seconds: phase.rem_euclid(period),
        coherence: r as f32,
        inliers,
        downbeat,
        ref_time: (beats[0] + beats[beats.len() - 1]) * 0.5,
    })
}

/// (resultant length, phase in seconds, bpm)
fn coherence(beats: &[f64], bpm: f64) -> (f64, f64, f64) {
    let period = 60.0 / bpm;
    let (mut sx, mut sy) = (0.0, 0.0);
    for &t in beats {
        let a = TAU * t / period;
        sx += a.cos();
        sy += a.sin();
    }
    let r = (sx * sx + sy * sy).sqrt() / beats.len() as f64;
    let phase = sy.atan2(sx) / TAU * period;
    (r, phase, bpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_tempo_from_quantized_beats() {
        // 160 BPM, beats quantized to 50 fps frames (what Beat This! outputs)
        let period = 60.0 / 160.0;
        let beats: Vec<f64> = (0..200).map(|k| ((0.137 + k as f64 * period) * 50.0).round() / 50.0).collect();
        let downbeats: Vec<f64> = beats.iter().skip(1).step_by(4).cloned().collect();
        let fit = fit_constant(&beats, &downbeats, BpmRange::default()).unwrap();
        assert!((fit.bpm - 160.0).abs() < 0.01, "bpm {}", fit.bpm);
        assert!(fit.inliers > 0.95);
        let (d, share) = fit.downbeat.unwrap();
        assert!((d - beats[1]).abs() < 0.03, "downbeat {}", d);
        assert!(share > 0.95);
    }
}
