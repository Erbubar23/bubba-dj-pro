use std::f32::consts::PI;

/// Direct Form II Transposed Biquad Filter for real-time DSP
#[derive(Debug, Clone)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

impl Biquad {
    fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    /// Creates a 2nd-order Butterworth low-pass filter
    pub fn lowpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self::lowpass_q(sample_rate, cutoff_hz, BUTTERWORTH_Q)
    }

    /// Creates a 2nd-order Butterworth high-pass filter
    pub fn highpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self::highpass_q(sample_rate, cutoff_hz, BUTTERWORTH_Q)
    }

    /// Low-pass with explicit resonance (Q > 0.707 adds a resonant peak)
    pub fn lowpass_q(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let mut f = Self::identity();
        f.set_lowpass(sample_rate, cutoff_hz, q);
        f
    }

    /// High-pass with explicit resonance
    pub fn highpass_q(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let mut f = Self::identity();
        f.set_highpass(sample_rate, cutoff_hz, q);
        f
    }

    /// Creates a constant 0dB peak gain bandpass filter (ideal for Kick drum isolation)
    pub fn bandpass(sample_rate: f32, center_hz: f32, q: f32) -> Self {
        let (cos_w0, alpha) = Self::prewarp(sample_rate, center_hz, q);
        let mut f = Self::identity();
        f.set_normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha);
        f
    }

    /// Updates coefficients in place while keeping the filter state,
    /// so sweeping the cutoff in real time does not click.
    pub fn set_lowpass(&mut self, sample_rate: f32, cutoff_hz: f32, q: f32) {
        let (cos_w0, alpha) = Self::prewarp(sample_rate, cutoff_hz, q);
        let b1 = 1.0 - cos_w0;
        self.set_normalized(b1 * 0.5, b1, b1 * 0.5, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha);
    }

    pub fn set_highpass(&mut self, sample_rate: f32, cutoff_hz: f32, q: f32) {
        let (cos_w0, alpha) = Self::prewarp(sample_rate, cutoff_hz, q);
        let b0 = (1.0 + cos_w0) * 0.5;
        self.set_normalized(b0, -(1.0 + cos_w0), b0, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha);
    }

    /// Clears the internal delay line
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn prewarp(sample_rate: f32, freq_hz: f32, q: f32) -> (f32, f32) {
        let w0 = 2.0 * PI * (freq_hz / sample_rate).clamp(0.0001, 0.499);
        (w0.cos(), w0.sin() / (2.0 * q.max(0.1)))
    }

    fn set_normalized(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let out = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * out + self.z2;
        self.z2 = self.b2 * input - self.a2 * out;
        out
    }

    pub fn process_buffer(&mut self, input: &[f32], output: &mut [f32]) {
        for (i, &sample) in input.iter().enumerate() {
            output[i] = self.process_sample(sample);
        }
    }
}
