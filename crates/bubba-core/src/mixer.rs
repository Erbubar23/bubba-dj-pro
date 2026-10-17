use std::f32::consts::PI;
use crate::deck::{Deck, DeckSnapshot};
use crate::sync::{self, SyncMode};
use bubba_analyzer::BeatGrid;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MixerSnapshot {
    pub deck1: DeckSnapshot,
    pub deck2: DeckSnapshot,
    pub crossfader: f32,
    /// Master knob position (0.0 .. 1.0, 0.8 = unity)
    pub master_volume: f32,
    pub master_vu_l: f32,
    pub master_vu_r: f32,
    pub headphone_mix: f32,
    pub headphone_volume: f32,
    /// Beat phase [0..1) of each deck, for the phase meter
    pub phase1: f32,
    pub phase2: f32,
    pub sync_mode: SyncMode,
}

pub struct Mixer {
    pub deck1: Deck,
    pub deck2: Deck,
    pub crossfader: f32,
    /// Master knob position (0.0 .. 1.0)
    pub master_volume: f32,
    master_gain: f32,
    pub sample_rate: f32,
    pub master_vu_l: f32,
    pub master_vu_r: f32,
    vu_decay: f32,

    // Headphone Cue Monitoring (PFL)
    pub headphone_mix: f32, // 0.0 = 100% Cue, 1.0 = 100% Master
    pub headphone_volume: f32,
    pub last_master: (f32, f32),
    /// rekordbox-style (absolute + pickup) or Serato-style (relative, "punto 0") sync
    pub sync_mode: SyncMode,
}

impl Mixer {
    pub fn new(sample_rate: f32) -> Self {
        let mut m = Self {
            deck1: Deck::new(sample_rate),
            deck2: Deck::new(sample_rate),
            crossfader: 0.0,
            master_volume: 0.8,
            master_gain: 1.0,
            sample_rate,
            master_vu_l: 0.0,
            master_vu_r: 0.0,
            vu_decay: 0.0,
            headphone_mix: 0.0, // Default to pure Cue in headphones
            headphone_volume: 0.8,
            last_master: (0.0, 0.0),
            sync_mode: SyncMode::default(),
        };
        m.set_sample_rate(sample_rate);
        m
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.deck1.set_output_rate(sample_rate);
        self.deck2.set_output_rate(sample_rate);
        self.vu_decay = (-1.0 / (0.25 * sample_rate)).exp();
    }

    pub fn deck_mut(&mut self, deck: usize) -> &mut Deck {
        if deck == 0 { &mut self.deck1 } else { &mut self.deck2 }
    }

    pub fn set_master_volume(&mut self, knob: f32) {
        self.master_volume = knob.clamp(0.0, 1.0);
        let g = self.master_volume / 0.8;
        self.master_gain = g * g;
    }

    /// (deck, other deck)
    fn pair_mut(&mut self, deck: usize) -> (&mut Deck, &mut Deck) {
        if deck == 0 { (&mut self.deck1, &mut self.deck2) } else { (&mut self.deck2, &mut self.deck1) }
    }

    /// Toggles beat sync on `follower_deck`; the other deck becomes the tempo master.
    /// Tempo is matched and the beat phase is aligned immediately (beat-grid based).
    pub fn trigger_sync(&mut self, follower_deck: usize) {
        let mode = self.sync_mode;
        let (follower, master) = self.pair_mut(follower_deck);
        if follower.is_sync_enabled {
            sync::disengage(follower, mode);
        } else if master.is_loaded() && follower.is_loaded() {
            sync::engage(master, follower, mode);
        }
    }

    /// Tempo fader: synced decks follow the active sync mode, the others move freely
    pub fn set_tempo(&mut self, deck: usize, value: f32) {
        let mode = self.sync_mode;
        let (this, other) = self.pair_mut(deck);
        if this.is_sync_enabled {
            sync::synced_fader(this, other, value, mode);
        } else {
            this.set_tempo_slider(value);
        }
    }

    /// Serato-style zero point reassignment (Shift + SYNC / 0% button):
    /// the current tempo becomes the new 0 %. Playback speed does NOT change.
    pub fn reassign_zero(&mut self, deck: usize) {
        let mode = self.sync_mode;
        let d = self.deck_mut(deck);
        if d.is_sync_enabled {
            sync::disengage(d, mode);
        }
        d.tempo.rezero();
    }

    /// Switching modes keeps every tempo as it is; it only changes how faders respond from now on
    pub fn set_sync_mode(&mut self, mode: SyncMode) {
        if self.sync_mode == mode {
            return;
        }
        self.sync_mode = mode;
        for d in [&mut self.deck1, &mut self.deck2] {
            if d.is_sync_enabled && mode == SyncMode::Relative {
                d.tempo.rezero();
            }
        }
    }

    /// Breaks any sync relation pointing at a deck whose track is being replaced
    pub fn release_sync_to(&mut self, deck: usize) {
        let mode = self.sync_mode;
        let (this, other) = self.pair_mut(deck);
        this.is_master = false;
        if other.is_sync_enabled {
            sync::disengage(other, mode);
        }
    }

    pub fn set_beatgrid(&mut self, deck: usize, grid: BeatGrid) {
        self.deck_mut(deck).set_beatgrid(grid);
    }

    pub fn get_snapshot(&self) -> MixerSnapshot {
        MixerSnapshot {
            deck1: self.deck1.get_snapshot(),
            deck2: self.deck2.get_snapshot(),
            crossfader: self.crossfader,
            master_volume: self.master_volume,
            master_vu_l: self.master_vu_l,
            master_vu_r: self.master_vu_r,
            headphone_mix: self.headphone_mix,
            headphone_volume: self.headphone_volume,
            phase1: self.deck1.beat_phase().unwrap_or(0.0) as f32,
            phase2: self.deck2.beat_phase().unwrap_or(0.0) as f32,
            sync_mode: self.sync_mode,
        }
    }

    /// Renders next master stereo sample: (Left, Right)
    #[inline]
    pub fn process_master(&mut self) -> (f32, f32) {
        if self.deck1.is_sync_enabled {
            sync::maintain(&self.deck2, &mut self.deck1);
        } else if self.deck2.is_sync_enabled {
            sync::maintain(&self.deck1, &mut self.deck2);
        }

        let (d1_l, d1_r) = self.deck1.process_sample();
        let (d2_l, d2_r) = self.deck2.process_sample();

        // DJ crossfader curve: both decks at full level in the centre, smooth fade to the edges
        let x = (self.crossfader + 1.0) * 0.5;
        let gain1 = (((1.0 - x) * PI * 0.5).sin() * std::f32::consts::SQRT_2).min(1.0);
        let gain2 = ((x * PI * 0.5).sin() * std::f32::consts::SQRT_2).min(1.0);

        let master_l = (d1_l * gain1 + d2_l * gain2) * self.master_gain;
        let master_r = (d1_r * gain1 + d2_r * gain2) * self.master_gain;

        self.master_vu_l = (self.master_vu_l * self.vu_decay).max(master_l.abs());
        self.master_vu_r = (self.master_vu_r * self.vu_decay).max(master_r.abs());

        let out = (Self::soft_clip(master_l), Self::soft_clip(master_r));
        self.last_master = out;
        out
    }

    /// Renders next headphone cue stereo sample: (Left, Right)
    #[inline]
    pub fn process_headphones(&self) -> (f32, f32) {
        let mut cue_l = 0.0f32;
        let mut cue_r = 0.0f32;

        if self.deck1.headphone_cue {
            cue_l += self.deck1.last_pfl.0;
            cue_r += self.deck1.last_pfl.1;
        }
        if self.deck2.headphone_cue {
            cue_l += self.deck2.last_pfl.0;
            cue_r += self.deck2.last_pfl.1;
        }

        // Blend between Cue and Master
        let cue_gain = 1.0 - self.headphone_mix;
        let master_gain = self.headphone_mix;
        let vol = self.headphone_volume * self.headphone_volume * 1.4;

        let hp_l = (cue_l * cue_gain + self.last_master.0 * master_gain) * vol;
        let hp_r = (cue_r * cue_gain + self.last_master.1 * master_gain) * vol;

        (Self::soft_clip(hp_l), Self::soft_clip(hp_r))
    }

    /// Transparent below 0.9, smooth (continuous slope) saturation above
    #[inline(always)]
    fn soft_clip(x: f32) -> f32 {
        const KNEE: f32 = 0.9;
        let a = x.abs();
        if a <= KNEE {
            x
        } else {
            let y = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
            y.copysign(x)
        }
    }
}
