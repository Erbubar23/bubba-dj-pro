use crate::filter::Biquad;

/// Onset envelopes at a ~1 kHz control rate (one value per `decimation` samples)
pub struct OnsetEnvelope {
    pub control_rate: f64,
    /// Positive derivative of the kick-band envelope (kick attacks)
    pub kick_flux: Vec<f32>,
    /// Positive derivative of the broadband (high-passed) envelope: snares, hats, crashes, chord changes
    pub full_flux: Vec<f32>,
    pub max_kick_flux: f32,
}

impl OnsetEnvelope {
    pub fn compute(mono: &[f32], sample_rate: u32) -> Self {
        let decimation = (sample_rate / 1000).max(1) as usize;
        let control_rate = sample_rate as f64 / decimation as f64;

        // Kick drum band (sub / low bass)
        let mut kick_filter = Biquad::bandpass(sample_rate as f32, 65.0, 1.8);
        // Everything above the bass
        let mut hp = Biquad::highpass(sample_rate as f32, 300.0);

        let frames = mono.len() / decimation;
        let mut kick_env = Vec::with_capacity(frames);
        let mut full_env = Vec::with_capacity(frames);
        for chunk in mono.chunks_exact(decimation) {
            let (mut k, mut f) = (0.0f32, 0.0f32);
            for &s in chunk {
                let a = kick_filter.process_sample(s);
                let b = hp.process_sample(s);
                k += a * a;
                f += b * b;
            }
            kick_env.push((k / decimation as f32).sqrt());
            full_env.push((f / decimation as f32).sqrt());
        }

        let kick_flux = positive_diff(&kick_env);
        let full_flux = positive_diff(&full_env);
        let max_kick_flux = kick_flux.iter().cloned().fold(0.0f32, f32::max);
        OnsetEnvelope { control_rate, kick_flux, full_flux, max_kick_flux }
    }

    pub fn len(&self) -> usize {
        self.kick_flux.len()
    }

    pub fn is_silent(&self) -> bool {
        self.max_kick_flux <= 1e-6
    }

    /// Strongest value of `flux` within ±`window` frames of `center`
    pub fn peak_near(flux: &[f32], center: f64, window: usize) -> f32 {
        let c = center.round() as isize;
        let start = (c - window as isize).max(0) as usize;
        let end = ((c + window as isize + 1).max(0) as usize).min(flux.len());
        if start >= end {
            return 0.0;
        }
        flux[start..end].iter().cloned().fold(0.0f32, f32::max)
    }

    /// Broadband attack strength right after every beat of a grid (frames).
    /// Tells the beat from the off-beat when kick-band energy cannot: in hard techno
    /// the off-beat rumble carries as much low end as the kick, but only the kick clicks.
    pub fn attack_strength(&self, phase: f64, period: f64) -> f32 {
        // A little before the candidate too: a beat time from the model can sit a few ms after the click
        let before = (0.025 * self.control_rate) as usize;
        let after = ((0.015 * self.control_rate) as usize).max(1);
        let mut sum = 0.0f32;
        let mut t = phase.rem_euclid(period) + period;
        while (t as usize) + after < self.full_flux.len() {
            let s = t as usize;
            sum += self.full_flux[s - before..s + after].iter().cloned().fold(0.0, f32::max);
            t += period;
        }
        sum
    }

    /// Sum of kick flux over consecutive blocks of `block_seconds` (for picking busy excerpts)
    pub fn block_energy(&self, block_seconds: f64) -> Vec<f32> {
        let block = ((block_seconds * self.control_rate) as usize).max(1);
        self.kick_flux.chunks(block).map(|c| c.iter().sum()).collect()
    }
}

fn positive_diff(env: &[f32]) -> Vec<f32> {
    let mut flux = vec![0.0f32; env.len()];
    for i in 1..env.len() {
        let d = env[i] - env[i - 1];
        if d > 0.0 {
            flux[i] = d;
        }
    }
    flux
}
