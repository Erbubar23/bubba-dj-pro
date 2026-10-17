use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;

use bubba_core::{AudioCommand, EqBand, StemType, SyncMode};
use bubba_midi::DjAction;

use crate::app::{App, RoutingRequest};
use crate::loader::load_track;

fn f32_of(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

fn bool_of(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

fn stem_of(s: &str) -> StemType {
    match s {
        "Vocal" => StemType::Vocal,
        "Drums" => StemType::Drums,
        _ => StemType::Bass,
    }
}

/// Commands coming from the web UI over the WebSocket
pub fn handle_ui_command(app: &Arc<App>, val: &Value) {
    let action = str_of(val, "action").unwrap_or_default();
    let deck = (val.get("deck").and_then(|d| d.as_u64()).unwrap_or(0) as usize).min(1);

    let cmd = match action {
        "PlayPause" => AudioCommand::PlayPause { deck },
        "Cue" => AudioCommand::Cue { deck, pressed: bool_of(val, "pressed").unwrap_or(true) },
        "Sync" => AudioCommand::Sync { deck },
        "HotCue" => AudioCommand::HotCue {
            deck,
            pad_index: val.get("pad_index").and_then(|p| p.as_u64()).unwrap_or(0) as usize,
            pressed: bool_of(val, "pressed").unwrap_or(true),
        },
        "DeleteHotCue" => AudioCommand::DeleteHotCue {
            deck,
            pad_index: val.get("pad_index").and_then(|p| p.as_u64()).unwrap_or(0) as usize,
        },
        "SetVolume" => match f32_of(val, "volume") {
            Some(volume) => AudioCommand::SetVolume { deck, volume },
            None => return,
        },
        "SetTrim" => match f32_of(val, "value") {
            Some(value) => AudioCommand::SetTrim { deck, value },
            None => return,
        },
        "SetTempo" => match f32_of(val, "value") {
            Some(value) => AudioCommand::SetTempo { deck, value },
            None => return,
        },
        "SetPitchRange" => match f32_of(val, "range") {
            Some(range) => AudioCommand::SetPitchRange { deck, range },
            None => return,
        },
        "SetCrossfader" => match f32_of(val, "value") {
            Some(value) => AudioCommand::SetCrossfader { value },
            None => return,
        },
        "SetMasterVolume" => match f32_of(val, "value") {
            Some(value) => AudioCommand::SetMasterVolume { value },
            None => return,
        },
        "SetEq" => {
            let band = match str_of(val, "band").unwrap_or("High") {
                "Low" => EqBand::Low,
                "Mid" => EqBand::Mid,
                _ => EqBand::High,
            };
            match f32_of(val, "value") {
                Some(value) => AudioCommand::SetEq { deck, band, value },
                None => return,
            }
        }
        "SetStemGain" => match (str_of(val, "stem"), f32_of(val, "gain")) {
            (Some(s), Some(gain)) => AudioCommand::SetStemGain { deck, stem: stem_of(s), gain },
            _ => return,
        },
        "HeadphoneCue" => AudioCommand::HeadphoneCue { deck, active: bool_of(val, "active") },
        "SetHeadphoneMix" => match f32_of(val, "mix") {
            Some(mix) => AudioCommand::SetHeadphoneMix { mix },
            None => return,
        },
        "SetHeadphoneVolume" => match f32_of(val, "volume") {
            Some(volume) => AudioCommand::SetHeadphoneVolume { volume },
            None => return,
        },
        "SetColorFx" => match f32_of(val, "value") {
            Some(value) => AudioCommand::SetColorFx { deck, value },
            None => return,
        },
        "SetLoop" => match f32_of(val, "beats") {
            Some(beats) => AudioCommand::SetLoop { deck, beats },
            None => return,
        },
        "ExitLoop" => AudioCommand::ExitLoop { deck },
        "LoopIn" => AudioCommand::LoopIn { deck },
        "LoopOut" => AudioCommand::LoopOut { deck },
        "ReloopExit" => AudioCommand::ReloopExit { deck },
        "LoopResize" => AudioCommand::LoopResize { deck, factor: f32_of(val, "factor").unwrap_or(1.0) },
        "ToggleSlip" => AudioCommand::ToggleSlip { deck },
        "BeatJump" => match val.get("beats").and_then(|b| b.as_i64()) {
            Some(beats) => AudioCommand::BeatJump { deck, beats: beats.clamp(-64, 64) as i32 },
            None => return,
        },
        "Seek" => match val.get("position").and_then(|p| p.as_f64()) {
            Some(position_seconds) => AudioCommand::Seek { deck, position_seconds },
            None => return,
        },
        "Jog" => AudioCommand::Jog { deck, delta: f32_of(val, "delta").unwrap_or(0.0) },
        "SetEchoFx" => AudioCommand::SetEchoFx {
            deck,
            active: bool_of(val, "active").unwrap_or(true),
            wet: f32_of(val, "wet").unwrap_or(0.5),
            beats: f32_of(val, "beats").unwrap_or(0.75),
        },
        "ReassignMasterTempo" => AudioCommand::ReassignMasterTempo { deck },
        "SetSyncMode" => {
            let mode = match str_of(val, "mode") {
                Some("Relative") => SyncMode::Relative,
                Some("Standard") => SyncMode::Standard,
                _ => return,
            };
            if let Ok(mut cfg) = app.config.lock() {
                cfg.sync_mode = mode;
                cfg.save();
            }
            AudioCommand::SetSyncMode { mode }
        }

        // ---- Application-level commands ----
        "LoadTrackFile" => {
            if let Some(path) = str_of(val, "path") {
                load_track(app, deck, PathBuf::from(path), bool_of(val, "force").unwrap_or(false));
            }
            return;
        }
        "SetAudioRouting" => {
            let master = str_of(val, "master").filter(|s| !s.is_empty()).map(String::from);
            // "" = explicitly no separate headphone device
            let headphone = str_of(val, "headphone").map(String::from);
            let _ = app.routing_tx.send(RoutingRequest::Set { master, headphone });
            return;
        }
        _ => return,
    };
    app.send(cmd);
}

/// Hardware events from the DDJ-FLX4
pub fn handle_dj_action(app: &Arc<App>, action: DjAction) {
    let cmd = match action {
        DjAction::DeckPlayPause { deck } => AudioCommand::PlayPause { deck },
        DjAction::DeckCue { deck, pressed } => AudioCommand::Cue { deck, pressed },
        DjAction::DeckSync { deck } => AudioCommand::Sync { deck },
        DjAction::DeckReassignTempo { deck } => AudioCommand::ReassignMasterTempo { deck },
        DjAction::DeckJogTouch { deck, pressed } => AudioCommand::JogTouch { deck, touched: pressed },
        DjAction::DeckJogRotate { deck, delta } => AudioCommand::Jog { deck, delta },
        DjAction::DeckJogScratch { deck, delta } => AudioCommand::Scratch { deck, delta },
        DjAction::DeckJogSearch { deck, delta } => AudioCommand::Search { deck, delta },
        DjAction::DeckTempoFader { deck, value } => AudioCommand::SetTempo { deck, value },
        DjAction::DeckVolumeFader { deck, value } => AudioCommand::SetVolume { deck, volume: value },
        DjAction::DeckTrim { deck, value } => AudioCommand::SetTrim { deck, value },
        DjAction::Crossfader { value } => AudioCommand::SetCrossfader { value },
        DjAction::DeckEqLow { deck, value } => AudioCommand::SetEq { deck, band: EqBand::Low, value },
        DjAction::DeckEqMid { deck, value } => AudioCommand::SetEq { deck, band: EqBand::Mid, value },
        DjAction::DeckEqHigh { deck, value } => AudioCommand::SetEq { deck, band: EqBand::High, value },
        DjAction::DeckHotCue { deck, pad_index, pressed } => AudioCommand::HotCue { deck, pad_index, pressed },
        DjAction::DeckHotCueDelete { deck, pad_index } => AudioCommand::DeleteHotCue { deck, pad_index },
        DjAction::DeckStemMuteToggle { deck, stem_index } => {
            let stem = match stem_index {
                0 => StemType::Vocal,
                1 => StemType::Drums,
                _ => StemType::Bass,
            };
            let current = app
                .snapshot()
                .map(|s| {
                    let d = if deck == 0 { s.deck1 } else { s.deck2 };
                    match stem {
                        StemType::Vocal => d.stem_vocal,
                        StemType::Drums => d.stem_drums,
                        StemType::Bass => d.stem_bass,
                    }
                })
                .unwrap_or(1.0);
            AudioCommand::SetStemGain { deck, stem, gain: if current > 0.5 { 0.0 } else { 1.0 } }
        }
        DjAction::HeadphoneCue { deck, active } => {
            if !active {
                return;
            }
            AudioCommand::HeadphoneCue { deck, active: None }
        }
        DjAction::DeckColorFx { deck, value } => AudioCommand::SetColorFx { deck, value },
        DjAction::DeckAutoLoop { deck, beats } => AudioCommand::SetLoop { deck, beats },
        DjAction::DeckExitLoop { deck } => AudioCommand::ExitLoop { deck },
        DjAction::DeckLoopIn { deck } => AudioCommand::LoopIn { deck },
        DjAction::DeckLoopOut { deck } => AudioCommand::LoopOut { deck },
        DjAction::DeckReloopExit { deck } => AudioCommand::ReloopExit { deck },
        DjAction::DeckLoopResize { deck, factor } => AudioCommand::LoopResize { deck, factor },
        DjAction::DeckSlipToggle { deck } => AudioCommand::ToggleSlip { deck },
        DjAction::DeckBeatJump { deck, beats } => AudioCommand::BeatJump { deck, beats },
        // Library navigation is handled by the UI (it owns the selection)
        DjAction::BrowseScroll { delta } => {
            app.hub.broadcast(&serde_json::json!({ "type": "midi_ui", "action": "browse", "delta": delta }));
            return;
        }
        DjAction::LoadSelected { deck } => {
            app.hub.broadcast(&serde_json::json!({ "type": "midi_ui", "action": "load", "deck": deck }));
            return;
        }
        DjAction::Raw { .. } => return,
    };
    app.send(cmd);
}
