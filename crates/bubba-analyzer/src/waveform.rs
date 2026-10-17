use crate::filter::Biquad;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default)]
pub struct WaveformFrame {
    /// Low frequency amplitude (Graves / Kick / Sub-bass -> Rojo) [0..255]
    pub low: u8,
    /// Mid frequency amplitude (Medios / Voces / Snare -> Verde) [0..255]
    pub mid: u8,
    /// High frequency amplitude (Agudos / Hi-Hats / Shakers -> Azul) [0..255]
    pub high: u8,
    /// Total peak envelope [0..255]
    pub peak: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WaveformData {
    pub frames_per_second: u32,
    pub frames: Vec<WaveformFrame>,
}

pub struct WaveformGenerator;

impl WaveformGenerator {
    /// Generates high-resolution 3-Band RGB waveform summary
    /// (Crossover frequencies: Low < 250 Hz, Mid 250 - 2500 Hz, High > 2500 Hz)
    pub fn generate(mono_samples: &[f32], sample_rate: u32, fps: u32) -> WaveformData {
        if mono_samples.is_empty() {
            return WaveformData {
                frames_per_second: fps,
                frames: Vec::new(),
            };
        }

        let samples_per_frame = (sample_rate / fps).max(1) as usize;
        let total_frames = mono_samples.len() / samples_per_frame;

        // 3-Band crossover filters
        let mut lp_filter = Biquad::lowpass(sample_rate as f32, 250.0);
        let mut hp_filter = Biquad::highpass(sample_rate as f32, 2500.0);

        let mut low_samples = vec![0.0f32; mono_samples.len()];
        let mut high_samples = vec![0.0f32; mono_samples.len()];
        let mut mid_samples = vec![0.0f32; mono_samples.len()];

        lp_filter.process_buffer(mono_samples, &mut low_samples);
        hp_filter.process_buffer(mono_samples, &mut high_samples);

        // Mid is whatever remains after removing low and high
        for i in 0..mono_samples.len() {
            mid_samples[i] = mono_samples[i] - low_samples[i] - high_samples[i];
        }

        let mut frames = Vec::with_capacity(total_frames);

        for frame_idx in 0..total_frames {
            let start = frame_idx * samples_per_frame;
            let end = (start + samples_per_frame).min(mono_samples.len());

            let mut max_low = 0.0f32;
            let mut max_mid = 0.0f32;
            let mut max_high = 0.0f32;
            let mut max_peak = 0.0f32;

            for i in start..end {
                let l = low_samples[i].abs();
                let m = mid_samples[i].abs();
                let h = high_samples[i].abs();
                let p = mono_samples[i].abs();

                if l > max_low { max_low = l; }
                if m > max_mid { max_mid = m; }
                if h > max_high { max_high = h; }
                if p > max_peak { max_peak = p; }
            }

            frames.push(WaveformFrame {
                low: (max_low.clamp(0.0, 1.0) * 255.0) as u8,
                mid: (max_mid.clamp(0.0, 1.0) * 255.0) as u8,
                high: (max_high.clamp(0.0, 1.0) * 255.0) as u8,
                peak: (max_peak.clamp(0.0, 1.0) * 255.0) as u8,
            });
        }

        WaveformData {
            frames_per_second: fps,
            frames,
        }
    }
}
