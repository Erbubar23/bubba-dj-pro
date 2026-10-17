//! Beat sync between two decks (beat-grid based).
//!
//! Two behaviours, switchable at runtime:
//! - `Standard`  rekordbox (Performance) / Pioneer: the synced deck follows the master and
//!   ignores its own tempo fader; after SYNC off the fader picks up (soft takeover) when it
//!   reaches the position of the current tempo.
//! - `Relative`  Serato DJ Pro relative pitch + "punto 0": syncing makes the synced tempo the
//!   new 0 %, the fader then works relative to it right away and moves every synced deck.

use crate::deck::Deck;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SyncMode {
    #[default]
    Standard,
    Relative,
}

/// Phase correction strength while synced (fraction of a beat corrected per beat)
const SYNC_GAIN: f64 = 0.15;
const SYNC_MAX_NUDGE: f64 = 0.01;

/// Follower tempo = master tempo × multiplier. Tracks at half / double tempo
/// (87 vs 174 BPM) are matched at their natural speed, like rekordbox and Serato do.
pub fn tempo_multiplier(master_bpm: f32, follower_track_bpm: f32) -> f64 {
    let ratio = (master_bpm.max(1.0) / follower_track_bpm.max(1.0)) as f64;
    [0.5, 1.0, 2.0]
        .into_iter()
        .min_by(|a, b| (ratio * a).ln().abs().total_cmp(&(ratio * b).ln().abs()))
        .unwrap_or(1.0)
}

fn wrap_phase(d: f64) -> f64 {
    d - d.round()
}

/// Beat phase error of the follower, in follower beats (-0.5 .. 0.5)
fn phase_error(master: &Deck, follower: &Deck, multiplier: f64) -> Option<f64> {
    let mb = master.beat_position()? * multiplier;
    let fb = follower.beat_position()?;
    Some(wrap_phase(mb.rem_euclid(1.0) - fb.rem_euclid(1.0)))
}

/// Locks `follower` to `master`: tempo matched and beat phase aligned immediately
pub fn engage(master: &mut Deck, follower: &mut Deck, mode: SyncMode) {
    master.is_master = true;
    if master.is_sync_enabled {
        disengage(master, mode);
    }
    let multiplier = tempo_multiplier(master.effective_bpm(), follower.track_bpm);
    let target = master.effective_bpm() as f64 * multiplier / follower.track_bpm.max(1.0) as f64;

    follower.is_sync_enabled = true;
    follower.is_master = false;
    follower.sync_multiplier = multiplier;
    follower.sync_nudge = 0.0;
    follower.tempo.set_rate_relative(target);
    if mode == SyncMode::Relative {
        follower.tempo.rezero();
    }

    if let Some(err) = phase_error(master, follower, multiplier) {
        let pos = follower.position + err * follower.beat_frames();
        follower.seek_frames(pos);
    }
}

pub fn disengage(deck: &mut Deck, mode: SyncMode) {
    deck.is_sync_enabled = false;
    deck.sync_nudge = 0.0;
    match mode {
        SyncMode::Standard => deck.tempo.begin_pickup(),
        SyncMode::Relative => {
            let rate = deck.tempo.rate;
            deck.tempo.set_rate_relative(rate);
        }
    }
}

/// Tempo fader of a synced deck (in Relative mode the master follows it)
pub fn synced_fader(follower: &mut Deck, master: &mut Deck, value: f32, mode: SyncMode) {
    match mode {
        SyncMode::Standard => follower.tempo.track_fader(value),
        SyncMode::Relative => {
            follower.tempo.move_fader_relative(value);
            // Every synced deck moves together: the master follows the new tempo
            let bpm = follower.effective_bpm() as f64 / follower.sync_multiplier;
            master.tempo.set_rate_relative(bpm / master.track_bpm.max(1.0) as f64);
        }
    }
}

/// Keeps the follower locked to the master's tempo and nudges its phase (per sample)
#[inline]
pub fn maintain(master: &Deck, follower: &mut Deck) {
    if !master.is_loaded() || !follower.is_loaded() {
        return;
    }
    follower.tempo.rate =
        master.effective_bpm() as f64 * follower.sync_multiplier / follower.track_bpm.max(1.0) as f64;

    follower.sync_nudge = 0.0;
    if master.is_playing && follower.is_playing {
        if let Some(err) = phase_error(master, follower, follower.sync_multiplier) {
            follower.sync_nudge = (err * SYNC_GAIN).clamp(-SYNC_MAX_NUDGE, SYNC_MAX_NUDGE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_and_double_tempo() {
        // 87 BPM track under a 174 BPM master plays at half the master's beat rate
        assert_eq!(tempo_multiplier(174.0, 87.0), 0.5);
        assert_eq!(tempo_multiplier(87.0, 174.0), 2.0);
        assert_eq!(tempo_multiplier(128.0, 126.0), 1.0);
        assert_eq!(tempo_multiplier(128.0, 100.0), 1.0);
        // Closer to half than to the same tempo: matched at half, like Serato / rekordbox
        assert_eq!(tempo_multiplier(155.0, 100.0), 0.5);
    }
}
