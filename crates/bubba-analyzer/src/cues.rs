//! Automatic cue point suggestions, in the spirit of rekordbox phrase analysis:
//! bar-aligned energy (bass + full band) per 4-bar block; big rises after a quiet
//! section are drops, big falls are breakdowns, the final fall is the outro.

use crate::beatgrid::{BeatGrid, BEATS_PER_BAR};
use crate::filter::Biquad;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CueKind {
    /// First beat with sound (rekordbox "auto cue")
    Start,
    Drop,
    Break,
    Outro,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CuePoint {
    pub seconds: f64,
    pub kind: CueKind,
}

const BARS_PER_BLOCK: usize = 4;
const MAX_CUES: usize = 8;
/// Minimum distance between two suggestions of the same kind
const MIN_GAP_BARS: usize = 16;

pub struct CueDetector;

impl CueDetector {
    pub fn suggest(mono: &[f32], sample_rate: u32, grid: &BeatGrid) -> Vec<CuePoint> {
        let mut cues = vec![CuePoint { seconds: grid.cue_seconds(), kind: CueKind::Start }];
        let sr = sample_rate as f64;
        let bar_seconds = grid.beat_interval_seconds * BEATS_PER_BAR as f64;
        if mono.is_empty() || !(bar_seconds > 0.0) {
            return cues;
        }

        // Blocks start on the bar of the first beat, so phrases line up with the music
        let first_bar = (grid.beat_at(grid.cue_seconds()) / BEATS_PER_BAR as f64).floor();
        let block_seconds = bar_seconds * BARS_PER_BLOCK as f64;
        let origin = grid.time_of_beat(first_bar * BEATS_PER_BAR as f64);
        let energy = block_energy(mono, sr, origin, block_seconds);
        if energy.len() < 6 {
            return cues;
        }

        let mut sorted = energy.clone();
        sorted.sort_by(f32::total_cmp);
        let high = sorted[sorted.len() * 4 / 5].max(1e-9);
        let level = |a: usize, b: usize| -> f32 {
            let (a, b) = (a.min(energy.len()), b.min(energy.len()));
            if a >= b { 0.0 } else { energy[a..b].iter().sum::<f32>() / (b - a) as f32 / high }
        };

        let mut last_drop: Option<usize> = None;
        let mut last_break: Option<usize> = None;
        let far = |last: Option<usize>, i: usize| last.map_or(true, |l| (i - l) * BARS_PER_BLOCK >= MIN_GAP_BARS);
        for i in 2..energy.len() - 1 {
            let before = level(i - 2, i);
            let after = level(i, i + 2);
            let t = origin + i as f64 * block_seconds;
            if before < 0.6 && after > 0.85 && after - before > 0.35 && far(last_drop, i) {
                cues.push(CuePoint { seconds: t, kind: CueKind::Drop });
                last_drop = Some(i);
            } else if before > 0.85 && after < 0.6 && before - after > 0.35 && far(last_break, i) {
                cues.push(CuePoint { seconds: t, kind: CueKind::Break });
                last_break = Some(i);
            }
        }

        // Outro: the last fall after which the energy stays low until the end
        if let Some(i) = (energy.len() / 2..energy.len() - 1)
            .rev()
            .find(|&i| level(i - 1, i) > 0.8 && level(i, energy.len()) < 0.6)
        {
            let t = origin + i as f64 * block_seconds;
            cues.retain(|c| !(c.kind == CueKind::Break && (c.seconds - t).abs() < block_seconds * 0.5));
            cues.push(CuePoint { seconds: t, kind: CueKind::Outro });
        }

        cues.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
        cues.truncate(MAX_CUES);
        cues
    }
}

/// Mean energy per block: low band (kick + bass) weighted over the full band
fn block_energy(mono: &[f32], sr: f64, origin: f64, block_seconds: f64) -> Vec<f32> {
    let mut low = Biquad::lowpass(sr as f32, 150.0);
    let start = (origin.max(0.0) * sr) as usize;
    let block = (block_seconds * sr) as usize;
    if block == 0 || start >= mono.len() {
        return Vec::new();
    }
    mono[start..]
        .chunks(block)
        .filter(|c| c.len() * 2 >= block)
        .map(|c| {
            let (mut l, mut f) = (0.0f64, 0.0f64);
            for &s in c {
                let x = low.process_sample(s) as f64;
                l += x * x;
                f += (s as f64) * (s as f64);
            }
            let n = c.len() as f64;
            (0.65 * (l / n).sqrt() + 0.35 * (f / n).sqrt()) as f32
        })
        .collect()
}
