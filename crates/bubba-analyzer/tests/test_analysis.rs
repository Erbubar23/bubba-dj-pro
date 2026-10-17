use bubba_analyzer::{BeatTracker, WaveformGenerator};

#[test]
fn test_downbeat_detection_with_intro_noise() {
    let sample_rate = 44100;
    let duration_seconds = 10.0;
    let total_samples = (sample_rate as f64 * duration_seconds) as usize;
    let mut mono = vec![0.0f32; total_samples];

    // 1. Simulate an intro with high-frequency noise / sweep (e.g. intro vocals or crash) in the first 1.5 seconds
    // This is the trap where Rekordbox and Mixxx fail!
    for i in 0..(sample_rate as usize * 3 / 2) {
        let t = i as f32 / sample_rate as f32;
        // High frequency hiss/sweep at 4000 Hz (NOT a kick drum)
        mono[i] = (2.0 * std::f32::consts::PI * 4000.0 * t).sin() * 0.8;
    }

    // 2. Real kick drum pattern starts at 2.0 seconds at 128.0 BPM (4/4 time)
    // Beat interval = 60 / 128 = 0.46875 seconds
    let target_bpm = 128.0f32;
    let beat_interval_sec = 60.0 / target_bpm;
    let start_offset_sec = 2.0f64;

    let mut beat_time = start_offset_sec;
    while beat_time < duration_seconds {
        let start_sample = (beat_time * sample_rate as f64) as usize;
        // Synthesize 50ms 60Hz sub-bass sine kick with decay
        let kick_samples = (sample_rate as f32 * 0.05) as usize;
        for k in 0..kick_samples {
            if start_sample + k < total_samples {
                let t = k as f32 / sample_rate as f32;
                let envelope = (1.0 - (k as f32 / kick_samples as f32)).powi(2);
                let kick_sine = (2.0 * std::f32::consts::PI * 60.0 * t).sin();
                mono[start_sample + k] += kick_sine * envelope * 0.95;
            }
        }
        beat_time += beat_interval_sec as f64;
    }

    // Run beat tracker
    let grid = BeatTracker::analyze(&mono, sample_rate);

    println!("Detected BPM: {}", grid.bpm);
    println!("Detected First Downbeat: {:.3}s (Target was {:.3}s)", grid.first_downbeat_seconds, start_offset_sec);

    // BPM should be very close to 128 (e.g. 127 - 129)
    assert!((grid.bpm - target_bpm).abs() < 2.5, "BPM should be near 128, got {}", grid.bpm);

    // Downbeat should NOT be at 0.0s (the noise intro was rejected!)
    assert!(grid.first_downbeat_seconds >= 1.5, "Downbeat must ignore intro noise, got {}", grid.first_downbeat_seconds);

    // Waveform check
    let wf = WaveformGenerator::generate(&mono, sample_rate, 150);
    assert!(!wf.frames.is_empty());
}

/// Same signal as generate_test_track.py: sweep intro, kicks from 2.0 s at 128 BPM,
/// noise hi-hats on every off-beat (the neural model tends to lock onto those)
fn kick_and_offbeat_hats(bpm: f64, seconds: f64) -> Vec<f32> {
    let sr = 44100.0;
    let mut mono = vec![0.0f32; (sr * seconds) as usize];
    for i in 0..(sr * 1.8) as usize {
        let t = i as f64 / sr;
        let freq = 1000.0 + 1500.0 * (t / 1.8);
        mono[i] += ((2.0 * std::f64::consts::PI * freq * t).sin() * 0.4) as f32;
    }
    let beat = 60.0 / bpm;
    let mut seed = 12345u32;
    let mut t = 2.0;
    while t < seconds {
        let start = (t * sr) as usize;
        let kick_len = (sr * 0.08) as usize;
        for k in 0..kick_len {
            if let Some(s) = mono.get_mut(start + k) {
                let env = (1.0 - k as f64 / kick_len as f64).powi(2);
                let freq = 120.0 * (1.0 - k as f64 / kick_len as f64) + 48.0;
                *s += ((2.0 * std::f64::consts::PI * freq * k as f64 / sr).sin() * env * 0.95) as f32;
            }
        }
        let hat = start + (beat * 0.5 * sr) as usize;
        let hat_len = (sr * 0.04) as usize;
        for h in 0..hat_len {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed >> 8) as f64 / (1u32 << 24) as f64 * 2.0 - 1.0;
            if let Some(s) = mono.get_mut(hat + h) {
                *s += (noise * (1.0 - h as f64 / hat_len as f64).powi(3) * 0.25) as f32;
            }
        }
        t += beat;
    }
    mono
}

#[test]
fn test_refined_grid_stays_on_the_kick() {
    let mono = kick_and_offbeat_hats(128.0, 15.0);
    let refined = bubba_analyzer::Analyzer::refine_beatgrid(&mono, 44100, &bubba_analyzer::AnalyzerConfig::default());
    let g = refined.beatgrid;
    println!("refined: {} BPM, first beat {:.3}s, bar 1 {:.3}s, {:?}", g.bpm, g.cue_seconds(), g.first_downbeat_seconds, g.source);
    assert!((g.bpm - 128.0).abs() < 0.01, "tempo must stay 128.00, got {}", g.bpm);
    assert!((g.cue_seconds() - 2.0).abs() < 0.03, "first beat must be the first kick (2.0 s), got {}", g.cue_seconds());
    // Every grid line lands on a kick, not on the hats in between
    let phase = (g.beat_at(2.0)).rem_euclid(1.0);
    assert!(phase < 0.06 || phase > 0.94, "grid is off the kicks by {} beats", phase);
}
