use bubba_core::{BeatGrid, DecodedAudio, Mixer, SyncMode};

fn silent_track(seconds: usize) -> DecodedAudio {
    DecodedAudio {
        samples: vec![0.0; 44100 * 2 * seconds],
        channels: 2,
        sample_rate: 44100,
        duration_seconds: seconds as f64,
    }
}

fn grid(bpm: f32, first: f64) -> Option<BeatGrid> {
    Some(BeatGrid::constant(bpm, first))
}

fn two_decks(bpm1: f32, bpm2: f32, mode: SyncMode) -> Mixer {
    let mut mixer = Mixer::new(44100.0);
    mixer.set_sync_mode(mode);
    mixer.deck1.load_track(silent_track(10), grid(bpm1, 0.0), None);
    mixer.deck2.load_track(silent_track(10), grid(bpm2, 0.0), None);
    mixer.deck1.is_playing = true;
    mixer.deck2.is_playing = true;
    mixer
}

fn assert_close(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 0.02, "{}: expected {}, got {}", what, b, a);
}

#[test]
fn test_serato_relative_sync_and_zero_reassignment() {
    let mut mixer = two_decks(128.0, 126.0, SyncMode::Standard);
    assert_eq!(mixer.deck2.get_snapshot().effective_bpm, 126.0);

    // 1. Sync Deck 2 (follower) to Deck 1
    mixer.trigger_sync(1);
    assert!(mixer.deck2.is_sync_enabled);
    assert!(mixer.deck1.is_master);
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 128.0, "synced bpm");

    // 2. Serato zero re-anchoring: 128 BPM becomes the new 0%, WITHOUT changing playback speed
    let rate_before = mixer.deck2.tempo.rate;
    mixer.reassign_zero(1);
    let snap = mixer.deck2.get_snapshot();
    assert_close(snap.base_bpm, 128.0, "0% point");
    assert_eq!(mixer.deck2.tempo.rate, rate_before, "playback speed must not jump");
    assert!(snap.pitch_percent.abs() < 1e-4);
    assert!(snap.tempo_pickup.is_none());

    // 3. Move pitch slider up (+0.5 on a -1..+1 slider, ±8% range) -> ramps from 128 without jumping
    mixer.set_tempo(1, 0.5);
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 128.0 * (1.0 + 0.5 * 0.08), "relative ramp");
}

#[test]
fn test_standard_sync_ignores_follower_fader_then_picks_up() {
    let mut mixer = two_decks(128.0, 126.0, SyncMode::Standard);
    mixer.set_tempo(1, 0.0);
    mixer.trigger_sync(1);

    // rekordbox: the synced deck's own fader does nothing
    mixer.set_tempo(1, -0.6);
    mixer.process_master();
    assert!(mixer.deck2.is_sync_enabled, "moving the fader must not break sync");
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 128.0, "fader ignored while synced");

    // The master's fader drives both decks
    mixer.set_tempo(0, 0.25); // +2 % of 128
    mixer.process_master();
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 130.56, "follower follows master");

    // SYNC off: tempo stays, fader needs to reach the pickup point (130.56 / 126 = +3.62 % -> 0.4524)
    mixer.trigger_sync(1);
    let pickup = mixer.deck2.get_snapshot().tempo_pickup.expect("pickup pending");
    assert!((pickup - 0.4524).abs() < 0.001, "pickup at {}", pickup);
    mixer.set_tempo(1, 0.1);
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 130.56, "no jump before pickup");
    mixer.set_tempo(1, 0.5); // crossed it: absolute again
    assert!(mixer.deck2.get_snapshot().tempo_pickup.is_none());
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 126.0 * 1.04, "absolute after pickup");
}

#[test]
fn test_relative_sync_sets_zero_point_and_moves_both_decks() {
    let mut mixer = two_decks(128.0, 126.0, SyncMode::Relative);
    mixer.set_tempo(1, -0.3); // fader anywhere: Serato does not care
    mixer.trigger_sync(1);

    let s = mixer.deck2.get_snapshot();
    assert_close(s.effective_bpm, 128.0, "synced bpm");
    assert_close(s.base_bpm, 128.0, "synced tempo is the new 0%");
    assert!(s.pitch_percent.abs() < 1e-3, "pitch reads 0.00% after sync, got {}", s.pitch_percent);

    // Fader up from where it is: +0.5 of travel = +4 % of 128, both decks, still synced
    mixer.set_tempo(1, 0.2);
    mixer.process_master();
    assert!(mixer.deck2.is_sync_enabled);
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 133.12, "follower ramps from 0 point");
    assert_close(mixer.deck1.get_snapshot().effective_bpm, 133.12, "master follows");
    assert_close(mixer.deck2.get_snapshot().pitch_percent, 4.0, "pitch reads +4%");

    // SYNC off: no pickup, the fader keeps working relative to the current tempo
    mixer.trigger_sync(1);
    assert!(mixer.deck2.get_snapshot().tempo_pickup.is_none());
    mixer.set_tempo(1, 0.3); // +0.1 of travel = +0.8 % of the 0 point (128)
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 133.12 + 128.0 * 0.008, "relative after sync off");
}

#[test]
fn test_sync_matches_half_and_double_tempo() {
    let mut mixer = two_decks(174.0, 87.0, SyncMode::Standard);
    mixer.trigger_sync(1);
    // 87 BPM track runs at its own speed against a 174 BPM master (half tempo)
    assert!((mixer.deck2.tempo.rate - 1.0).abs() < 1e-6, "rate {}", mixer.deck2.tempo.rate);
    mixer.set_tempo(0, 0.5); // master +4 %
    mixer.process_master();
    assert_close(mixer.deck2.get_snapshot().effective_bpm, 87.0 * 1.04, "half-tempo follower");
}

#[test]
fn test_sync_aligns_beat_phase() {
    let mut mixer = Mixer::new(44100.0);
    mixer.deck1.load_track(silent_track(20), grid(128.0, 0.5), None);
    mixer.deck2.load_track(silent_track(20), grid(126.0, 1.0), None);
    mixer.deck1.seek_seconds(3.3);
    mixer.deck2.seek_seconds(7.1);

    mixer.trigger_sync(1);
    let p1 = mixer.deck1.beat_phase().unwrap();
    let p2 = mixer.deck2.beat_phase().unwrap();
    let diff = (p1 - p2).abs();
    assert!(diff < 1e-3 || (1.0 - diff) < 1e-3, "phases should match: {} vs {}", p1, p2);

    // Keep both playing for 2 seconds: they must stay locked
    mixer.deck1.is_playing = true;
    mixer.deck2.is_playing = true;
    for _ in 0..(44100 * 2) {
        mixer.process_master();
    }
    let p1 = mixer.deck1.beat_phase().unwrap();
    let p2 = mixer.deck2.beat_phase().unwrap();
    let diff = (p1 - p2).abs();
    assert!(diff < 0.01 || (1.0 - diff) < 0.01, "drifted: {} vs {}", p1, p2);
}

#[test]
fn test_refined_grid_keeps_speed_and_moves_default_cue() {
    let mut mixer = Mixer::new(44100.0);
    mixer.deck1.load_track(silent_track(20), grid(77.5, 0.2), None);
    mixer.deck1.set_tempo_slider(0.25);
    let speed = mixer.deck1.tempo.rate;

    let mut refined = BeatGrid::constant(155.0, 0.5);
    refined.first_beat_seconds = Some(0.5);
    mixer.set_beatgrid(0, refined);

    let s = mixer.deck1.get_snapshot();
    assert_eq!(mixer.deck1.tempo.rate, speed, "playback speed unchanged");
    assert_close(s.effective_bpm, 155.0 * 1.02, "bpm display doubles");
    assert!((s.cue_position_seconds - 0.5).abs() < 1e-6, "default cue follows the new first beat");
    assert!((s.position_seconds - 0.5).abs() < 1e-6, "parked playhead follows the cue");
}

#[test]
fn test_loop_and_beat_jump_use_track_time() {
    let mut mixer = Mixer::new(48000.0); // device rate differs from the 44.1 kHz file
    mixer.deck1.load_track(silent_track(20), grid(120.0, 0.0), None);
    mixer.deck1.set_tempo_slider(1.0); // +8 %
    mixer.deck1.seek_seconds(2.0);
    mixer.deck1.set_loop(4.0);
    let s = mixer.deck1.get_snapshot();
    assert!(s.loop_active);
    // 4 beats at 120 BPM = 2.0 s of track time regardless of pitch
    assert!(((s.loop_end_seconds - s.loop_start_seconds) - 2.0).abs() < 1e-6);

    mixer.deck1.exit_loop();
    mixer.deck1.seek_seconds(4.0);
    mixer.deck1.beat_jump(8);
    assert!((mixer.deck1.get_snapshot().position_seconds - 8.0).abs() < 1e-6);

    // Playback at 48 kHz output: one second of output = 1.08 s of track at +8 %
    mixer.deck1.seek_seconds(1.0);
    mixer.deck1.is_playing = true;
    for _ in 0..48000 {
        mixer.process_master();
    }
    let pos = mixer.deck1.get_snapshot().position_seconds;
    assert!((pos - 2.08).abs() < 0.001, "expected 2.08 s, got {}", pos);
}
