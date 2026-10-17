use bubba_analyzer::filter::Biquad;

const CFX_DEADZONE: f32 = 0.02;
const CFX_RESONANCE: f32 = 1.1;

#[derive(Debug, Clone, Copy, PartialEq)]
enum FilterMode {
    Bypass,
    LowPass,
    HighPass,
}

/// Pioneer DJM-style Sound Color FX (Bipolar Resonant Filter)
#[derive(Debug, Clone)]
pub struct ColorFxFilter {
    sample_rate: f32,
    pub value: f32, // -1.0 (LPF) .. 0.0 (Bypass) .. +1.0 (HPF)
    mode: FilterMode,
    filter_l: Biquad,
    filter_r: Biquad,
}

impl ColorFxFilter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            value: 0.0,
            mode: FilterMode::Bypass,
            filter_l: Biquad::lowpass(sample_rate, 20000.0),
            filter_r: Biquad::lowpass(sample_rate, 20000.0),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        let v = self.value;
        self.mode = FilterMode::Bypass;
        self.set_value(v);
    }

    pub fn set_value(&mut self, val: f32) {
        let v = val.clamp(-1.0, 1.0);
        self.value = v;

        let new_mode = if v < -CFX_DEADZONE {
            FilterMode::LowPass
        } else if v > CFX_DEADZONE {
            FilterMode::HighPass
        } else {
            FilterMode::Bypass
        };

        // Entering a mode from another one: start from a clean delay line
        if new_mode != self.mode {
            self.filter_l.reset();
            self.filter_r.reset();
            self.mode = new_mode;
        }

        match new_mode {
            FilterMode::LowPass => {
                // Sweeps cutoff logarithmically from 20 kHz down to ~63 Hz
                let norm = (-v - CFX_DEADZONE) / (1.0 - CFX_DEADZONE);
                let cutoff = (20000.0 * 10.0f32.powf(-2.5 * norm)).max(50.0);
                self.filter_l.set_lowpass(self.sample_rate, cutoff, CFX_RESONANCE);
                self.filter_r.set_lowpass(self.sample_rate, cutoff, CFX_RESONANCE);
            }
            FilterMode::HighPass => {
                // Sweeps cutoff from 20 Hz up to ~12 kHz
                let norm = (v - CFX_DEADZONE) / (1.0 - CFX_DEADZONE);
                let cutoff = (20.0 * 10.0f32.powf(2.77 * norm)).min(18000.0);
                self.filter_l.set_highpass(self.sample_rate, cutoff, CFX_RESONANCE);
                self.filter_r.set_highpass(self.sample_rate, cutoff, CFX_RESONANCE);
            }
            FilterMode::Bypass => {}
        }
    }

    #[inline(always)]
    pub fn process(&mut self, input_l: f32, input_r: f32) -> (f32, f32) {
        if self.mode == FilterMode::Bypass {
            return (input_l, input_r); // Bypass zero-cost
        }

        (
            self.filter_l.process_sample(input_l),
            self.filter_r.process_sample(input_r),
        )
    }
}

/// Tempo-Synchronized Beat Echo / Delay FX.
/// When switched off the echo tail keeps ringing out naturally instead of cutting abruptly.
#[derive(Debug, Clone)]
pub struct BeatEchoFx {
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    sample_rate: f32,
    idle_samples: usize,
    pub active: bool,
    pub wet: f32, // 0.0 to 1.0
    pub feedback: f32,
    pub beat_fraction: f32, // e.g. 0.5 (1/2 beat), 0.75 (3/4 beat), 1.0 (1 beat)
}

impl BeatEchoFx {
    pub fn new(sample_rate: f32) -> Self {
        let max_samples = (sample_rate * 4.0) as usize; // up to 4 seconds buffer
        Self {
            buffer_l: vec![0.0; max_samples],
            buffer_r: vec![0.0; max_samples],
            write_pos: 0,
            sample_rate,
            idle_samples: usize::MAX,
            active: false,
            wet: 0.5,
            feedback: 0.55,
            beat_fraction: 0.75, // 3/4 beat default (classic DJ delay)
        }
    }

    pub fn set_active(&mut self, active: bool) {
        if active && !self.active {
            self.idle_samples = 0;
        }
        self.active = active;
    }

    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32, effective_bpm: f32) -> (f32, f32) {
        let len = self.buffer_l.len();
        if !self.active && self.idle_samples > len {
            return (in_l, in_r);
        }

        let bpm = effective_bpm.max(40.0);
        let beat_sec = (60.0 / bpm) * self.beat_fraction;
        let delay_samples = ((beat_sec * self.sample_rate) as usize).clamp(1, len - 1);
        let read_pos = (self.write_pos + len - delay_samples) % len;

        let delayed_l = self.buffer_l[read_pos];
        let delayed_r = self.buffer_r[read_pos];

        let (send_l, send_r) = if self.active { (in_l, in_r) } else { (0.0, 0.0) };
        let new_l = (send_l + delayed_l * self.feedback).clamp(-1.5, 1.5);
        let new_r = (send_r + delayed_r * self.feedback).clamp(-1.5, 1.5);

        self.buffer_l[self.write_pos] = new_l;
        self.buffer_r[self.write_pos] = new_r;
        self.write_pos = (self.write_pos + 1) % len;

        if !self.active {
            if new_l.abs() < 1e-5 && new_r.abs() < 1e-5 {
                self.idle_samples += 1;
            } else {
                self.idle_samples = 0;
            }
        }

        (in_l + delayed_l * self.wet, in_r + delayed_r * self.wet)
    }
}
