use bubba_analyzer::{DecodedAudio, BeatGrid, KeyResult, filter::Biquad};
use crate::commands::{EqBand, StemType};
use crate::fx::{ColorFxFilter, BeatEchoFx};
use crate::tempo::TempoControl;

/// Audio covered by one full platter revolution at 33⅓ RPM
const SECONDS_PER_REV: f64 = 1.8;
/// Jog wheel resolution of the DDJ-FLX4 / DDJ-400 (ticks per revolution)
const JOG_TICKS_PER_REV: f64 = 720.0;
/// Smoothing time of the scratch follower (keeps scratches free of zipper noise)
const SCRATCH_SMOOTHING_SECONDS: f64 = 0.006;
/// Peak meter release time
const VU_RELEASE_SECONDS: f32 = 0.25;

/// Telemetry sent to the UI ~100 times per second. `Copy` so the audio thread never allocates.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DeckSnapshot {
    pub loaded: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    /// Native tempo of the track (from the beat grid)
    pub track_bpm: f32,
    /// Tempo at the 0% pitch reference point
    pub base_bpm: f32,
    pub effective_bpm: f32,
    pub pitch_percent: f32,
    pub pitch_range: f32,
    pub tempo_fader: f32,
    /// Tempo fader position to reach before it takes over again (soft takeover after sync)
    pub tempo_pickup: Option<f32>,
    /// Current playback speed in track-seconds per second (for UI interpolation)
    pub speed: f32,
    pub is_playing: bool,
    pub is_scratching: bool,
    pub is_sync_enabled: bool,
    pub is_master: bool,
    pub volume: f32,
    pub trim: f32,
    pub eq_low: f32,
    pub eq_mid: f32,
    pub eq_high: f32,
    pub stem_vocal: f32,
    pub stem_drums: f32,
    pub stem_bass: f32,
    pub cue_position_seconds: f64,
    pub hot_cues_seconds: [Option<f64>; 8],
    pub vu_l: f32,
    pub vu_r: f32,
    pub headphone_cue: bool,

    // Pro DJ Features Telemetry
    pub color_fx: f32,
    pub is_slip_active: bool,
    pub slip_position_seconds: f64,
    pub loop_active: bool,
    pub loop_size_beats: f32,
    pub loop_start_seconds: f64,
    pub loop_end_seconds: f64,
    pub loop_in_seconds: Option<f64>,
    pub echo_active: bool,
    pub echo_wet: f32,
    pub echo_beats: f32,
}

pub struct Deck {
    pub audio: Option<DecodedAudio>,
    pub beatgrid: Option<BeatGrid>,
    pub key: Option<KeyResult>,
    /// Playhead in track frames (source sample rate)
    pub position: f64,
    pub is_playing: bool,
    pub cue_position: f64,
    cue_previewing: bool,
    pub hot_cues: [Option<f64>; 8],

    /// Tempo fader, pitch range and playback rate (see `tempo.rs`)
    pub tempo: TempoControl,
    /// Tiny phase correction applied while synced
    pub sync_nudge: f64,
    /// Follower tempo = master tempo × this (0.5 / 1 / 2: half / double tempo matching)
    pub sync_multiplier: f64,
    pub jog_bend: f64,

    // Vinyl scratch
    scratching: bool,
    scratch_pending: f64,
    scratch_coeff: f64,
    last_advance: f64,

    pub volume: f32,
    trim_knob: f32,
    trim_gain: f32,
    /// Native BPM of the loaded track
    pub track_bpm: f32,
    pub is_sync_enabled: bool,
    pub is_master: bool,
    pub headphone_cue: bool,
    /// Output (device) sample rate
    pub sample_rate: f32,
    resample_ratio: f64,

    pub last_pfl: (f32, f32),

    // 3-Band EQ (knob position + resulting gain)
    eq_low_knob: f32,
    eq_mid_knob: f32,
    eq_high_knob: f32,
    eq_low_gain: f32,
    eq_mid_gain: f32,
    eq_high_gain: f32,

    // Real-Time Stems Isolation (1.0 = normal, 0.0 = muted)
    pub stem_vocal_gain: f32,
    pub stem_drums_gain: f32,
    pub stem_bass_gain: f32,

    // Pro DJ Tools
    pub color_fx: ColorFxFilter,
    pub echo_fx: BeatEchoFx,
    pub is_slip_active: bool,
    pub slip_position: f64,
    pub loop_active: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub loop_size_beats: f32,
    loop_in_point: Option<f64>,

    // VU meter peak envelope
    pub vu_peak_l: f32,
    pub vu_peak_r: f32,
    vu_decay: f32,

    // Filters for EQ & Vocal Stems
    filter_low_l: Biquad,
    filter_low_r: Biquad,
    filter_high_l: Biquad,
    filter_high_r: Biquad,
    vocal_bandpass_m: Biquad,
}

fn eq_gain(knob: f32) -> f32 {
    let v = knob.clamp(0.0, 1.0);
    if v <= 0.5 { v / 0.5 } else { 1.0 + (v - 0.5) / 0.5 }
}

fn trim_gain(knob: f32) -> f32 {
    let v = knob.clamp(0.0, 1.0);
    if v <= 0.001 {
        return 0.0;
    }
    // -24 dB .. 0 dB (centre) .. +9 dB, like a DJM trim
    let db = if v < 0.5 { -24.0 * (1.0 - v / 0.5) } else { 9.0 * (v - 0.5) / 0.5 };
    10.0f32.powf(db / 20.0)
}

/// 4-point Hermite interpolation
#[inline(always)]
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

impl Deck {
    pub fn new(sample_rate: f32) -> Self {
        let mut deck = Self {
            audio: None,
            beatgrid: None,
            key: None,
            position: 0.0,
            is_playing: false,
            cue_position: 0.0,
            cue_previewing: false,
            hot_cues: [None; 8],
            tempo: TempoControl::new(),
            sync_nudge: 0.0,
            sync_multiplier: 1.0,
            jog_bend: 0.0,
            scratching: false,
            scratch_pending: 0.0,
            scratch_coeff: 0.0,
            last_advance: 0.0,
            volume: 1.0,
            trim_knob: 0.5,
            trim_gain: 1.0,
            track_bpm: 120.0,
            is_sync_enabled: false,
            is_master: false,
            headphone_cue: false,
            sample_rate,
            resample_ratio: 1.0,
            last_pfl: (0.0, 0.0),

            eq_low_knob: 0.5,
            eq_mid_knob: 0.5,
            eq_high_knob: 0.5,
            eq_low_gain: 1.0,
            eq_mid_gain: 1.0,
            eq_high_gain: 1.0,

            stem_vocal_gain: 1.0,
            stem_drums_gain: 1.0,
            stem_bass_gain: 1.0,

            color_fx: ColorFxFilter::new(sample_rate),
            echo_fx: BeatEchoFx::new(sample_rate),
            is_slip_active: false,
            slip_position: 0.0,
            loop_active: false,
            loop_start: 0.0,
            loop_end: 0.0,
            loop_size_beats: 4.0,
            loop_in_point: None,

            vu_peak_l: 0.0,
            vu_peak_r: 0.0,
            vu_decay: 0.0,

            filter_low_l: Biquad::lowpass(sample_rate, 250.0),
            filter_low_r: Biquad::lowpass(sample_rate, 250.0),
            filter_high_l: Biquad::highpass(sample_rate, 2500.0),
            filter_high_r: Biquad::highpass(sample_rate, 2500.0),
            vocal_bandpass_m: Biquad::bandpass(sample_rate, 1200.0, 0.8),
        };
        deck.set_output_rate(sample_rate);
        deck
    }

    /// Re-targets all DSP to a new output device sample rate (called when re-routing audio)
    pub fn set_output_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.filter_low_l = Biquad::lowpass(sample_rate, 250.0);
        self.filter_low_r = Biquad::lowpass(sample_rate, 250.0);
        self.filter_high_l = Biquad::highpass(sample_rate, 2500.0);
        self.filter_high_r = Biquad::highpass(sample_rate, 2500.0);
        self.vocal_bandpass_m = Biquad::bandpass(sample_rate, 1200.0, 0.8);
        self.color_fx.set_sample_rate(sample_rate);
        let echo = &self.echo_fx;
        let (active, wet, feedback, beats) = (echo.active, echo.wet, echo.feedback, echo.beat_fraction);
        self.echo_fx = BeatEchoFx::new(sample_rate);
        self.echo_fx.set_active(active);
        self.echo_fx.wet = wet;
        self.echo_fx.feedback = feedback;
        self.echo_fx.beat_fraction = beats;
        self.scratch_coeff = 1.0 / (SCRATCH_SMOOTHING_SECONDS * sample_rate as f64).max(1.0);
        self.vu_decay = (-1.0 / (VU_RELEASE_SECONDS * sample_rate)).exp();
        self.update_resample_ratio();
    }

    fn update_resample_ratio(&mut self) {
        let src = self.audio.as_ref().map(|a| a.sample_rate as f64).unwrap_or(self.sample_rate as f64);
        self.resample_ratio = src / self.sample_rate.max(1.0) as f64;
    }

    /// Loads a new track. Returns the previous audio buffer so the caller can free it
    /// outside the real-time thread.
    pub fn load_track(&mut self, audio: DecodedAudio, beatgrid: Option<BeatGrid>, key: Option<KeyResult>) -> Option<DecodedAudio> {
        if let Some(ref bg) = beatgrid {
            self.track_bpm = bg.bpm.max(1.0);
            self.cue_position = bg.cue_seconds() * audio.sample_rate as f64;
        } else {
            self.track_bpm = 120.0;
            self.cue_position = 0.0;
        }
        self.beatgrid = beatgrid;
        self.key = key;
        let old = self.audio.replace(audio);
        self.update_resample_ratio();

        self.position = self.cue_position;
        self.slip_position = self.cue_position;
        self.is_playing = false;
        self.cue_previewing = false;
        self.scratching = false;
        self.scratch_pending = 0.0;
        self.jog_bend = 0.0;
        self.hot_cues = [None; 8];
        self.loop_active = false;
        self.loop_in_point = None;
        self.last_pfl = (0.0, 0.0);

        // Tempo goes back to the absolute fader position
        self.is_sync_enabled = false;
        self.is_master = false;
        self.sync_nudge = 0.0;
        self.sync_multiplier = 1.0;
        self.tempo.reset_absolute();
        old
    }

    /// Replaces the grid of the loaded track (refined analysis or user edit).
    /// Playback speed is kept; an untouched default CUE moves to the new first beat.
    pub fn set_beatgrid(&mut self, grid: BeatGrid) {
        if self.audio.is_none() {
            return;
        }
        let sr = self.src_rate();
        let old_cue = self.beatgrid.as_ref().map(|g| g.cue_seconds() * sr);
        let cue_untouched = old_cue.map_or(true, |c| (self.cue_position - c).abs() < 2.0);
        let parked_on_cue = !self.is_playing && (self.position - self.cue_position).abs() < 2.0;

        self.track_bpm = grid.bpm.max(1.0);
        if cue_untouched {
            self.cue_position = grid.cue_seconds() * sr;
            if parked_on_cue {
                self.seek_frames(self.cue_position);
            }
        }
        self.beatgrid = Some(grid);
    }

    pub fn is_loaded(&self) -> bool {
        self.audio.is_some()
    }

    fn src_rate(&self) -> f64 {
        self.audio.as_ref().map(|a| a.sample_rate as f64).unwrap_or(44100.0)
    }

    fn total_frames(&self) -> f64 {
        self.audio.as_ref().map(|a| a.total_frames() as f64).unwrap_or(0.0)
    }

    /// Length of one beat in track frames (independent of the playback rate)
    pub fn beat_frames(&self) -> f64 {
        let beat_sec = self
            .beatgrid
            .as_ref()
            .map(|g| g.beat_interval_seconds)
            .filter(|b| *b > 0.0)
            .unwrap_or(60.0 / self.track_bpm.max(1.0) as f64);
        beat_sec * self.src_rate()
    }

    fn grid_anchor(&self) -> f64 {
        self.beatgrid.as_ref().map(|g| g.first_downbeat_seconds * self.src_rate()).unwrap_or(0.0)
    }

    /// Snaps a position to the nearest beat of the grid
    fn quantize(&self, pos: f64) -> f64 {
        if self.beatgrid.is_none() {
            return pos;
        }
        let bf = self.beat_frames();
        let anchor = self.grid_anchor();
        anchor + ((pos - anchor) / bf).round() * bf
    }

    /// Continuous beat count from the grid anchor (0.0 = a downbeat)
    pub fn beat_position(&self) -> Option<f64> {
        self.audio.as_ref()?;
        Some((self.position - self.grid_anchor()) / self.beat_frames())
    }

    /// Fractional position inside the current beat [0, 1)
    pub fn beat_phase(&self) -> Option<f64> {
        self.beat_position().map(|b| b.rem_euclid(1.0))
    }

    pub fn effective_bpm(&self) -> f32 {
        self.track_bpm * self.tempo.rate as f32
    }

    fn slip_displaced(&self) -> bool {
        self.loop_active || self.scratching
    }

    /// Moves the playhead. Leaving the loop region cancels the loop.
    pub fn seek_frames(&mut self, frames: f64) {
        let total = self.total_frames();
        self.position = frames.clamp(0.0, (total - 1.0).max(0.0));
        if self.loop_active && (self.position < self.loop_start || self.position >= self.loop_end) {
            self.loop_active = false;
        }
        if !self.slip_displaced() {
            self.slip_position = self.position;
        }
    }

    pub fn seek_seconds(&mut self, seconds: f64) {
        if self.audio.is_some() {
            let f = seconds * self.src_rate();
            self.seek_frames(f);
        }
    }

    pub fn toggle_play_pause(&mut self) {
        if self.audio.is_none() {
            return;
        }
        if self.cue_previewing {
            // PLAY while holding CUE: keep playing after CUE is released (CDJ behaviour)
            self.cue_previewing = false;
            self.is_playing = true;
            return;
        }
        self.is_playing = !self.is_playing;
        if self.is_playing && self.position >= self.total_frames() - 2.0 {
            self.seek_frames(self.cue_position);
        }
    }

    /// CDJ-style CUE: while playing returns to the cue point and stops; while paused
    /// sets the cue point (if moved) and previews while held.
    pub fn cue(&mut self, pressed: bool) {
        if self.audio.is_none() {
            return;
        }
        if pressed {
            if self.is_playing && !self.cue_previewing {
                self.is_playing = false;
                self.seek_frames(self.cue_position);
            } else if !self.is_playing {
                if (self.position - self.cue_position).abs() > 2.0 {
                    self.cue_position = self.position;
                }
                self.is_playing = true;
                self.cue_previewing = true;
            }
        } else if self.cue_previewing {
            self.cue_previewing = false;
            self.is_playing = false;
            self.seek_frames(self.cue_position);
        }
    }

    /// Jog ring: pitch bend while playing, fine search while paused
    pub fn jog_rotate(&mut self, delta: f32) {
        if self.audio.is_none() {
            return;
        }
        if self.is_playing {
            self.jog_bend = (self.jog_bend + delta as f64 * 0.02).clamp(-0.9, 0.9);
        } else {
            let frames_per_tick = self.src_rate() * SECONDS_PER_REV / JOG_TICKS_PER_REV;
            self.seek_frames(self.position + delta as f64 * frames_per_tick);
        }
    }

    pub fn jog_touch(&mut self, touched: bool) {
        if self.audio.is_none() {
            return;
        }
        if touched && !self.scratching {
            self.scratching = true;
            self.scratch_pending = 0.0;
            self.jog_bend = 0.0;
        } else if !touched && self.scratching {
            self.scratching = false;
            self.scratch_pending = 0.0;
            if self.is_slip_active && self.is_playing {
                self.position = self.slip_position;
            }
            if !self.slip_displaced() {
                self.slip_position = self.position;
            }
        }
    }

    pub fn scratch(&mut self, delta: f32) {
        if self.scratching {
            self.scratch_pending += delta as f64 * self.src_rate() * SECONDS_PER_REV / JOG_TICKS_PER_REV;
        } else {
            self.jog_rotate(delta);
        }
    }

    pub fn search(&mut self, delta: f32) {
        if self.audio.is_some() {
            // Shift + jog: one tick = 1/8 beat
            let step = self.beat_frames() / 8.0;
            self.seek_frames(self.position + delta as f64 * step);
        }
    }

    /// Tempo fader on a deck that is not synced (synced decks go through `Mixer::set_tempo`)
    pub fn set_tempo_slider(&mut self, value: f32) {
        self.tempo.move_fader(value);
    }

    pub fn set_pitch_range(&mut self, range: f32) {
        self.tempo.set_range(range);
    }

    pub fn set_eq(&mut self, band: EqBand, value: f32) {
        let knob = value.clamp(0.0, 1.0);
        let gain = eq_gain(knob);
        match band {
            EqBand::Low => { self.eq_low_knob = knob; self.eq_low_gain = gain; }
            EqBand::Mid => { self.eq_mid_knob = knob; self.eq_mid_gain = gain; }
            EqBand::High => { self.eq_high_knob = knob; self.eq_high_gain = gain; }
        }
    }

    pub fn set_trim(&mut self, value: f32) {
        self.trim_knob = value.clamp(0.0, 1.0);
        self.trim_gain = trim_gain(self.trim_knob);
    }

    pub fn set_stem_gain(&mut self, stem: StemType, gain: f32) {
        let g = gain.clamp(0.0, 1.0);
        match stem {
            StemType::Vocal => self.stem_vocal_gain = g,
            StemType::Drums => self.stem_drums_gain = g,
            StemType::Bass => self.stem_bass_gain = g,
        }
    }

    pub fn trigger_hot_cue(&mut self, pad_idx: usize, pressed: bool) {
        if pad_idx >= 8 || !pressed || self.audio.is_none() {
            return;
        }
        if let Some(pos) = self.hot_cues[pad_idx] {
            self.cue_previewing = false;
            self.seek_frames(pos);
            self.is_playing = true;
        } else {
            let pos = if self.is_playing { self.quantize(self.position) } else { self.position };
            self.hot_cues[pad_idx] = Some(pos.max(0.0));
        }
    }

    pub fn delete_hot_cue(&mut self, pad_idx: usize) {
        if pad_idx < 8 {
            self.hot_cues[pad_idx] = None;
        }
    }

    // ==========================================
    // PRO DJ METHODS: Auto Loop, Slip, Beat Jump
    // ==========================================

    fn wrap_into_loop(&mut self) {
        let len = self.loop_end - self.loop_start;
        if len > 0.0 && self.position >= self.loop_end {
            self.position = self.loop_start + (self.position - self.loop_start) % len;
        }
    }

    /// Quantized auto loop. Pressing the active size again exits; another size resizes.
    pub fn set_loop(&mut self, beats: f32) {
        if self.audio.is_none() {
            return;
        }
        let beats = beats.clamp(1.0 / 32.0, 64.0);
        let len = beats as f64 * self.beat_frames();

        if self.loop_active {
            if (self.loop_size_beats - beats).abs() < 1e-4 {
                self.exit_loop();
                return;
            }
            self.loop_end = self.loop_start + len;
            self.loop_size_beats = beats;
            self.wrap_into_loop();
            return;
        }

        let start = self.quantize(self.position);
        self.loop_start = start.max(0.0);
        self.loop_end = self.loop_start + len;
        self.loop_size_beats = beats;
        self.loop_active = true;
        self.wrap_into_loop();
    }

    pub fn exit_loop(&mut self) {
        if self.loop_active {
            self.loop_active = false;
            // If Slip mode was on during the loop, return to the background playhead
            if self.is_slip_active && self.is_playing {
                self.position = self.slip_position;
            }
            if !self.slip_displaced() {
                self.slip_position = self.position;
            }
        }
    }

    pub fn loop_in(&mut self) {
        if self.audio.is_some() {
            self.loop_in_point = Some(self.quantize(self.position).max(0.0));
        }
    }

    pub fn loop_out(&mut self) {
        let Some(start) = self.loop_in_point else { return };
        let bf = self.beat_frames();
        let mut end = self.quantize(self.position);
        if end <= start + bf * 0.1 {
            end = start + bf;
        }
        self.loop_start = start;
        self.loop_end = end;
        self.loop_size_beats = ((end - start) / bf) as f32;
        self.loop_active = true;
        self.loop_in_point = None;
        self.wrap_into_loop();
    }

    pub fn reloop_exit(&mut self) {
        if self.loop_active {
            self.exit_loop();
        } else if self.loop_end > self.loop_start {
            self.loop_active = true;
            self.position = self.loop_start;
        }
    }

    pub fn loop_resize(&mut self, factor: f32) {
        let new_size = (self.loop_size_beats * factor).clamp(1.0 / 32.0, 64.0);
        if self.loop_active {
            self.loop_end = self.loop_start + new_size as f64 * self.beat_frames();
            self.wrap_into_loop();
        }
        self.loop_size_beats = new_size;
    }

    pub fn toggle_slip(&mut self) {
        self.is_slip_active = !self.is_slip_active;
        if self.is_slip_active && !self.slip_displaced() {
            self.slip_position = self.position;
        }
    }

    /// Jumps by whole beats keeping the phase (moves an active loop along with it)
    pub fn beat_jump(&mut self, beats: i32) {
        if self.audio.is_none() {
            return;
        }
        let jump = beats as f64 * self.beat_frames();
        let max = (self.total_frames() - 1.0).max(0.0);
        if self.loop_active {
            self.loop_start += jump;
            self.loop_end += jump;
        }
        self.position = (self.position + jump).clamp(0.0, max);
        if self.slip_displaced() {
            self.slip_position = (self.slip_position + jump).clamp(0.0, max);
        } else {
            self.slip_position = self.position;
        }
    }

    pub fn get_snapshot(&self) -> DeckSnapshot {
        let sr = self.src_rate();
        let dur_sec = self.audio.as_ref().map(|a| a.duration_seconds).unwrap_or(0.0);

        let mut hot_cues_sec = [None; 8];
        for i in 0..8 {
            hot_cues_sec[i] = self.hot_cues[i].map(|f| f / sr);
        }

        let speed = if self.scratching || self.is_playing {
            (self.last_advance / self.resample_ratio) as f32
        } else {
            0.0
        };

        DeckSnapshot {
            loaded: self.audio.is_some(),
            position_seconds: self.position / sr,
            duration_seconds: dur_sec,
            track_bpm: self.track_bpm,
            base_bpm: self.track_bpm * self.tempo.zero_rate as f32,
            effective_bpm: self.effective_bpm(),
            pitch_percent: self.tempo.percent() as f32,
            pitch_range: self.tempo.range,
            tempo_fader: self.tempo.fader,
            tempo_pickup: self.tempo.pickup_position(),
            speed,
            is_playing: self.is_playing,
            is_scratching: self.scratching,
            is_sync_enabled: self.is_sync_enabled,
            is_master: self.is_master,
            volume: self.volume,
            trim: self.trim_knob,
            eq_low: self.eq_low_knob,
            eq_mid: self.eq_mid_knob,
            eq_high: self.eq_high_knob,
            stem_vocal: self.stem_vocal_gain,
            stem_drums: self.stem_drums_gain,
            stem_bass: self.stem_bass_gain,
            cue_position_seconds: self.cue_position / sr,
            hot_cues_seconds: hot_cues_sec,
            vu_l: self.vu_peak_l,
            vu_r: self.vu_peak_r,
            headphone_cue: self.headphone_cue,

            color_fx: self.color_fx.value,
            is_slip_active: self.is_slip_active,
            slip_position_seconds: self.slip_position / sr,
            loop_active: self.loop_active,
            loop_size_beats: self.loop_size_beats,
            loop_start_seconds: self.loop_start / sr,
            loop_end_seconds: self.loop_end / sr,
            loop_in_seconds: self.loop_in_point.map(|f| f / sr),
            echo_active: self.echo_fx.active,
            echo_wet: self.echo_fx.wet,
            echo_beats: self.echo_fx.beat_fraction,
        }
    }

    #[inline(always)]
    fn read_frame(audio: &DecodedAudio, idx: isize, total: isize) -> (f32, f32) {
        let i = idx.clamp(0, total - 1) as usize;
        if audio.channels >= 2 {
            let ch = audio.channels as usize;
            (audio.samples[i * ch], audio.samples[i * ch + 1])
        } else {
            let v = audio.samples[i];
            (v, v)
        }
    }

    /// Renders next stereo sample frame: returns (Left, Right)
    #[inline]
    pub fn process_sample(&mut self) -> (f32, f32) {
        self.vu_peak_l *= self.vu_decay;
        self.vu_peak_r *= self.vu_decay;

        let audio = match self.audio {
            Some(ref a) => a,
            None => {
                self.last_pfl = (0.0, 0.0);
                return (0.0, 0.0);
            }
        };

        let normal_advance = (self.tempo.rate * (1.0 + self.sync_nudge)).max(0.0) * self.resample_ratio;
        let advance = if self.scratching {
            let step = self.scratch_pending * self.scratch_coeff;
            self.scratch_pending -= step;
            step
        } else if self.is_playing {
            (normal_advance + self.jog_bend * self.resample_ratio).max(0.0)
        } else {
            self.last_advance = 0.0;
            self.last_pfl = (0.0, 0.0);
            return (0.0, 0.0);
        };
        self.last_advance = advance;

        let total = audio.total_frames() as isize;
        if total < 4 {
            return (0.0, 0.0);
        }

        let frame_index = self.position.floor() as isize;
        if frame_index >= total - 1 && advance >= 0.0 && !self.scratching {
            self.is_playing = false;
            self.position = (total - 1) as f64;
            self.last_pfl = (0.0, 0.0);
            return (0.0, 0.0);
        }

        let t = (self.position - frame_index as f64) as f32;
        let (a_l, a_r) = Self::read_frame(audio, frame_index - 1, total);
        let (b_l, b_r) = Self::read_frame(audio, frame_index, total);
        let (c_l, c_r) = Self::read_frame(audio, frame_index + 1, total);
        let (d_l, d_r) = Self::read_frame(audio, frame_index + 2, total);
        let mut raw_l = hermite(a_l, b_l, c_l, d_l, t);
        let mut raw_r = hermite(a_r, b_r, c_r, d_r, t);

        if self.scratching {
            // Fade out when the platter is (almost) still, avoids DC thumps
            let motion = ((advance.abs() / self.resample_ratio) * 20.0).min(1.0) as f32;
            raw_l *= motion;
            raw_r *= motion;
        }

        // Advance playhead(s)
        self.position = (self.position + advance).clamp(0.0, (total - 1) as f64);
        if self.slip_displaced() {
            if self.is_playing {
                self.slip_position += normal_advance;
            }
        }

        // Loop boundary
        if self.loop_active && !self.scratching && self.position >= self.loop_end {
            let len = (self.loop_end - self.loop_start).max(1.0);
            self.position = self.loop_start + (self.position - self.loop_end) % len;
        }

        if !self.slip_displaced() {
            self.slip_position = self.position;
        }

        if self.jog_bend.abs() > 0.00001 {
            self.jog_bend *= 0.9996;
        } else {
            self.jog_bend = 0.0;
        }

        // Trim
        raw_l *= self.trim_gain;
        raw_r *= self.trim_gain;

        // Stems Mid/Side processing
        let mut mid = (raw_l + raw_r) * 0.5;
        let side = (raw_l - raw_r) * 0.5;

        let vocal_band = self.vocal_bandpass_m.process_sample(mid);
        if self.stem_vocal_gain < 1.0 {
            mid -= (1.0 - self.stem_vocal_gain) * vocal_band;
        }

        let stem_l = mid + side;
        let stem_r = mid - side;

        let low_l = self.filter_low_l.process_sample(stem_l);
        let low_r = self.filter_low_r.process_sample(stem_r);
        let high_l = self.filter_high_l.process_sample(stem_l);
        let high_r = self.filter_high_r.process_sample(stem_r);
        let mid_l = stem_l - low_l - high_l;
        let mid_r = stem_r - low_r - high_r;

        let final_low_gain = self.eq_low_gain * self.stem_bass_gain;
        let final_high_gain = self.eq_high_gain * self.stem_drums_gain;

        let base_l = low_l * final_low_gain + mid_l * self.eq_mid_gain + high_l * final_high_gain;
        let base_r = low_r * final_low_gain + mid_r * self.eq_mid_gain + high_r * final_high_gain;

        // Sound Color FX (Bipolar Filter)
        let (cfx_l, cfx_r) = self.color_fx.process(base_l, base_r);

        // Pre-Fade Listen (PFL)
        self.last_pfl = (cfx_l, cfx_r);

        // Channel fader (squared curve feels like a real mixer)
        let fader_gain = self.volume * self.volume;
        let fader_l = cfx_l * fader_gain;
        let fader_r = cfx_r * fader_gain;

        // Post-fader Beat Echo (tails keep ringing after the fader closes)
        let eff_bpm = self.effective_bpm();
        let (out_l, out_r) = self.echo_fx.process(fader_l, fader_r, eff_bpm);

        if out_l.abs() > self.vu_peak_l { self.vu_peak_l = out_l.abs(); }
        if out_r.abs() > self.vu_peak_r { self.vu_peak_r = out_r.abs(); }

        (out_l, out_r)
    }
}
