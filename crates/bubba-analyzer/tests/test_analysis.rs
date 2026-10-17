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
