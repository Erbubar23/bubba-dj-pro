use std::f32::consts::PI;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeyResult {
    /// Standard Musical Key name, e.g. "A minor", "C major"
    pub key_name: String,
    /// Camelot Wheel notation, e.g. "8A", "8B"
    pub camelot: String,
    /// Harmonic confidence score (0.0 .. 1.0)
    pub confidence: f32,
}

pub struct KeyDetector;

/// Analysis rate after decimation (~11 kHz is plenty for C2..B5 fundamentals)
const TARGET_RATE: u32 = 11025;
/// ~186 ms windows give ~5 Hz resolution, enough to separate semitones down to C3
const WINDOW: usize = 2048;
const HOP: usize = 2048;

impl KeyDetector {
    /// Detects musical key and Camelot Wheel notation using chroma correlation
    /// against Krumhansl-Kessler profiles. Uses Goertzel filters (no trig in the inner loop).
    pub fn analyze(mono_samples: &[f32], sample_rate: u32) -> KeyResult {
        let decimation = (sample_rate / TARGET_RATE).max(1) as usize;
        let rate = sample_rate as f32 / decimation as f32;

        // Box-filter decimation (cheap anti-aliasing)
        let signal: Vec<f32> = mono_samples
            .chunks_exact(decimation)
            .map(|c| c.iter().sum::<f32>() / decimation as f32)
            .collect();

        if signal.len() < WINDOW {
            return KeyResult {
                key_name: "--".to_string(),
                camelot: "--".to_string(),
                confidence: 0.0,
            };
        }

        // Precompute Goertzel coefficients and a Hann window
        let notes: Vec<(usize, f32)> = (48..=83)
            .map(|midi: i32| {
                let freq = 440.0f32 * 2.0f32.powf((midi as f32 - 69.0) / 12.0);
                let coeff = 2.0 * (2.0 * PI * freq / rate).cos();
                ((midi % 12) as usize, coeff)
            })
            .collect();
        let hann: Vec<f32> = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / (WINDOW - 1) as f32).cos())
            .collect();

        let mut chroma = [0.0f32; 12];
        let mut windowed = vec![0.0f32; WINDOW];

        let mut start = 0;
        while start + WINDOW <= signal.len() {
            let frame = &signal[start..start + WINDOW];
            let mut energy = 0.0f32;
            for i in 0..WINDOW {
                windowed[i] = frame[i] * hann[i];
                energy += windowed[i] * windowed[i];
            }
            start += HOP;
            if energy < 1e-6 {
                continue; // skip silence
            }

            let mut frame_chroma = [0.0f32; 12];
            for &(pitch_class, coeff) in &notes {
                let (mut s1, mut s2) = (0.0f32, 0.0f32);
                for &x in &windowed {
                    let s0 = x + coeff * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
                frame_chroma[pitch_class] += power.max(0.0).sqrt();
            }

            // Normalise per frame so loud sections don't dominate
            let sum: f32 = frame_chroma.iter().sum();
            if sum > 0.0 {
                for (c, f) in chroma.iter_mut().zip(frame_chroma.iter()) {
                    *c += f / sum;
                }
            }
        }

        let chroma_sum: f32 = chroma.iter().sum();
        if chroma_sum <= 0.0 {
            return KeyResult {
                key_name: "--".to_string(),
                camelot: "--".to_string(),
                confidence: 0.0,
            };
        }
        for c in chroma.iter_mut() {
            *c /= chroma_sum;
        }

        // Krumhansl-Kessler Key Profiles
        let major_profile = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
        let minor_profile = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

        let note_names = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
        let camelot_major = ["8B", "3B", "10B", "5B", "12B", "7B", "2B", "9B", "4B", "11B", "6B", "1B"];
        let camelot_minor = ["5A", "12A", "7A", "2A", "9A", "4A", "11A", "6A", "1A", "8A", "3A", "10A"];

        // Pearson correlation for each of the 24 keys
        let mut scores = Vec::with_capacity(24);
        for shift in 0..12 {
            let rotated: Vec<f32> = (0..12).map(|i| chroma[(i + shift) % 12]).collect();
            scores.push((Self::correlation(&rotated, &major_profile), shift, false));
            scores.push((Self::correlation(&rotated, &minor_profile), shift, true));
        }
        scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let (best_score, best_note_idx, best_is_minor) = scores[0];
        let runner_up = scores[1].0;

        let key_name = format!("{} {}", note_names[best_note_idx], if best_is_minor { "minor" } else { "major" });
        let camelot = if best_is_minor {
            camelot_minor[best_note_idx].to_string()
        } else {
            camelot_major[best_note_idx].to_string()
        };

        KeyResult {
            key_name,
            camelot,
            confidence: (best_score.max(0.0) * 0.6 + (best_score - runner_up).max(0.0) * 4.0).clamp(0.0, 1.0),
        }
    }

    fn correlation(x: &[f32], y: &[f32; 12]) -> f32 {
        let mx = x.iter().sum::<f32>() / 12.0;
        let my = y.iter().sum::<f32>() / 12.0;
        let (mut num, mut dx, mut dy) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..12 {
            let a = x[i] - mx;
            let b = y[i] - my;
            num += a * b;
            dx += a * a;
            dy += b * b;
        }
        if dx <= 0.0 || dy <= 0.0 {
            0.0
        } else {
            num / (dx.sqrt() * dy.sqrt())
        }
    }
}
