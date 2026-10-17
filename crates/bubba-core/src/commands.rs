use bubba_analyzer::{DecodedAudio, BeatGrid, KeyResult};
use crate::sync::SyncMode;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EqBand {
    Low,
    Mid,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StemType {
    Vocal,
    Drums,
    Bass,
}

pub enum AudioCommand {
    LoadTrack {
        deck: usize,
        audio: DecodedAudio,
        beatgrid: Option<BeatGrid>,
        key: Option<KeyResult>,
    },
    PlayPause { deck: usize },
    Cue { deck: usize, pressed: bool },
    /// Jog ring / pitch bend (relative ticks)
    Jog { deck: usize, delta: f32 },
    /// Jog platter touched (vinyl mode scratch)
    JogTouch { deck: usize, touched: bool },
    /// Platter rotation while touched (relative ticks)
    Scratch { deck: usize, delta: f32 },
    /// Fast search (Shift + jog)
    Search { deck: usize, delta: f32 },
    /// Tempo fader position (-1.0 .. +1.0)
    SetTempo { deck: usize, value: f32 },
    /// Tempo fader range (0.06, 0.08, 0.16, 0.50 ...)
    SetPitchRange { deck: usize, range: f32 },
    SetVolume { deck: usize, volume: f32 },
    /// Channel trim knob (0.0 .. 1.0, 0.5 = 0 dB)
    SetTrim { deck: usize, value: f32 },
    /// EQ knob (0.0 = kill, 0.5 = flat, 1.0 = +6 dB)
    SetEq { deck: usize, band: EqBand, value: f32 },
    SetStemGain { deck: usize, stem: StemType, gain: f32 },
    Sync { deck: usize },
    SetCrossfader { value: f32 },
    /// Master level knob (0.0 .. 1.0, 0.8 = unity)
    SetMasterVolume { value: f32 },
    HotCue { deck: usize, pad_index: usize, pressed: bool },
    DeleteHotCue { deck: usize, pad_index: usize },
    Seek { deck: usize, position_seconds: f64 },
    HeadphoneCue { deck: usize, active: Option<bool> },
    SetHeadphoneMix { mix: f32 },
    SetHeadphoneVolume { volume: f32 },

    // Professional DJ Features
    SetColorFx { deck: usize, value: f32 }, // -1.0 (LPF) .. 0.0 .. +1.0 (HPF)
    /// Quantized auto loop; pressing the same size again exits the loop
    SetLoop { deck: usize, beats: f32 },
    ExitLoop { deck: usize },
    LoopIn { deck: usize },
    LoopOut { deck: usize },
    ReloopExit { deck: usize },
    /// Multiply the active loop length (0.5 = halve, 2.0 = double)
    LoopResize { deck: usize, factor: f32 },
    ToggleSlip { deck: usize },
    BeatJump { deck: usize, beats: i32 }, // -32 .. +32 beats
    SetEchoFx { deck: usize, active: bool, wet: f32, beats: f32 },
    ReassignMasterTempo { deck: usize },
    /// rekordbox-style or Serato-style ("punto 0") sync behaviour
    SetSyncMode { mode: SyncMode },
    /// Replaces the grid of the loaded track (refined analysis or user edit)
    SetBeatGrid { deck: usize, grid: BeatGrid },
}
