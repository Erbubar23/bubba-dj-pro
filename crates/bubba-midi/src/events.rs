#[derive(Debug, Clone, PartialEq)]
pub enum DjAction {
    /// Toggle Play/Pause on Deck (0 = Deck 1, 1 = Deck 2)
    DeckPlayPause { deck: usize },
    /// Cue button pressed or released
    DeckCue { deck: usize, pressed: bool },
    /// Sync button toggled
    DeckSync { deck: usize },
    /// Jog wheel top platter touched (Scratch mode engaged)
    DeckJogTouch { deck: usize, pressed: bool },
    /// Jog ring rotated (pitch bend). Positive = forward
    DeckJogRotate { deck: usize, delta: f32 },
    /// Jog platter rotated while touched (scratch). Positive = forward
    DeckJogScratch { deck: usize, delta: f32 },
    /// Shift + jog: fast search
    DeckJogSearch { deck: usize, delta: f32 },
    /// Pitch / Tempo slider (-1.0 = slowest, 0.0 = 0%, +1.0 = fastest)
    DeckTempoFader { deck: usize, value: f32 },
    /// Channel Volume fader (0.0 = silence, 1.0 = unity)
    DeckVolumeFader { deck: usize, value: f32 },
    /// Trim knob (0.0 .. 1.0, 0.5 = 0 dB)
    DeckTrim { deck: usize, value: f32 },
    /// EQ Low knob (0.0 = kill, 0.5 = 0dB flat, 1.0 = +6dB boost)
    DeckEqLow { deck: usize, value: f32 },
    /// EQ Mid knob
    DeckEqMid { deck: usize, value: f32 },
    /// EQ High knob
    DeckEqHigh { deck: usize, value: f32 },
    /// Color FX Knob (-1.0 = LPF .. 0.0 = Flat .. +1.0 = HPF)
    DeckColorFx { deck: usize, value: f32 },
    /// Performance Pad (Hot Cue 0..7)
    DeckHotCue { deck: usize, pad_index: usize, pressed: bool },
    /// Shift + Hot Cue pad: delete
    DeckHotCueDelete { deck: usize, pad_index: usize },
    /// Stem Mute / Solo Toggle (0 = Vocal, 1 = Drums, 2 = Bass)
    DeckStemMuteToggle { deck: usize, stem_index: usize },
    /// Auto Loop (e.g. 4 beats, 8 beats)
    DeckAutoLoop { deck: usize, beats: f32 },
    /// Exit active loop
    DeckExitLoop { deck: usize },
    DeckLoopIn { deck: usize },
    DeckLoopOut { deck: usize },
    DeckReloopExit { deck: usize },
    /// Halve (0.5) or double (2.0) the loop
    DeckLoopResize { deck: usize, factor: f32 },
    /// Slip Mode toggle
    DeckSlipToggle { deck: usize },
    /// Beat Jump forward/backward (-32 .. +32 beats)
    DeckBeatJump { deck: usize, beats: i32 },
    /// Crossfader (-1.0 = 100% Deck 1, 0.0 = Center, +1.0 = 100% Deck 2)
    Crossfader { value: f32 },
    /// Headphone Cue Monitor toggle
    HeadphoneCue { deck: usize, active: bool },
    /// Serato-Style Master Tempo / Zero Point Reassignment
    DeckReassignTempo { deck: usize },
    /// Library browse knob rotated (+ = down)
    BrowseScroll { delta: i32 },
    /// LOAD button: load the selected library track
    LoadSelected { deck: usize },
    /// Raw unmapped event for debugging/custom mapping
    Raw { status: u8, data1: u8, data2: u8 },
}
