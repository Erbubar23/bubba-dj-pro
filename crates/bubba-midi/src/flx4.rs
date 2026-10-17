use crate::events::DjAction;

/// Pioneer DDJ-FLX4 MIDI layout (same family as the DDJ-400):
///
/// * MIDI ch 1 / 2  (0x90/0xB0, 0x91/0xB1): Deck 1 / Deck 2 transport, jog, tempo, channel strip
/// * MIDI ch 7      (0x96/0xB6): mixer section (crossfader, CFX knobs, browser, LOAD)
/// * MIDI ch 8 / 10 (0x97, 0x99): performance pads of Deck 1 / Deck 2
/// * MIDI ch 9 / 11 (0x98, 0x9A): same pads with SHIFT held
pub struct Flx4Parser;

/// Beat jump pad layout: top row backwards / forwards pairs
const BEAT_JUMP_PADS: [i32; 8] = [-1, 1, -2, 2, -4, 4, -8, 8];
/// Beat loop pad sizes
const BEAT_LOOP_PADS: [f32; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];

#[inline]
fn relative(data2: u8) -> f32 {
    // Pioneer relative encoding: 0x40 = 0, 0x41 = +1, 0x3F = -1
    data2 as f32 - 64.0
}

#[inline]
fn bipolar(data2: u8) -> f32 {
    ((data2 as f32 / 63.5) - 1.0).clamp(-1.0, 1.0)
}

#[inline]
fn unipolar(data2: u8) -> f32 {
    data2 as f32 / 127.0
}

impl Flx4Parser {
    /// Translates raw MIDI bytes [status, data1, data2] from a Pioneer DDJ-FLX4 into a DjAction
    pub fn parse(bytes: &[u8]) -> Option<DjAction> {
        if bytes.len() < 3 {
            return None;
        }

        let (status, data1, data2) = (bytes[0], bytes[1], bytes[2]);
        let msg_type = status & 0xF0;
        let channel = status & 0x0F;
        let raw = Some(DjAction::Raw { status, data1, data2 });

        // Note Off is equivalent to Note On with velocity 0
        let (is_note, pressed) = match msg_type {
            0x90 => (true, data2 > 0),
            0x80 => (true, false),
            _ => (false, false),
        };

        if is_note {
            return match channel {
                0 | 1 => Self::deck_note(channel as usize, data1, pressed).or(raw),
                6 => Self::mixer_note(data1, pressed).or(raw),
                7 | 9 => Self::pad_note(if channel == 7 { 0 } else { 1 }, data1, pressed, false).or(raw),
                8 | 10 => Self::pad_note(if channel == 8 { 0 } else { 1 }, data1, pressed, true).or(raw),
                _ => raw,
            };
        }

        if msg_type == 0xB0 {
            return match channel {
                0 | 1 => Self::deck_cc(channel as usize, data1, data2).or(raw),
                6 => Self::mixer_cc(data1, data2).or(raw),
                _ => raw,
            };
        }

        raw
    }

    fn deck_note(deck: usize, note: u8, pressed: bool) -> Option<DjAction> {
        let on = |a: DjAction| if pressed { Some(a) } else { None };
        match note {
            0x0B => on(DjAction::DeckPlayPause { deck }),
            0x0C => Some(DjAction::DeckCue { deck, pressed }),
            0x58 => on(DjAction::DeckSync { deck }),
            // Shift + Sync (Serato Zero Point Reassignment)
            0x59 | 0x5C | 0x60 => on(DjAction::DeckReassignTempo { deck }),
            0x36 | 0x67 => Some(DjAction::DeckJogTouch { deck, pressed }),
            0x54 => Some(DjAction::HeadphoneCue { deck, active: pressed }),
            // Loop section
            0x10 => on(DjAction::DeckLoopIn { deck }),
            0x11 => on(DjAction::DeckLoopOut { deck }),
            0x4D => on(DjAction::DeckReloopExit { deck }),
            0x50 => on(DjAction::DeckAutoLoop { deck, beats: 4.0 }),
            0x51 => on(DjAction::DeckLoopResize { deck, factor: 0.5 }),
            0x53 => on(DjAction::DeckLoopResize { deck, factor: 2.0 }),
            _ => None,
        }
    }

    fn mixer_note(note: u8, pressed: bool) -> Option<DjAction> {
        if !pressed {
            return None;
        }
        match note {
            0x46 => Some(DjAction::LoadSelected { deck: 0 }),
            0x47 => Some(DjAction::LoadSelected { deck: 1 }),
            _ => None,
        }
    }

    fn pad_note(deck: usize, note: u8, pressed: bool, shift: bool) -> Option<DjAction> {
        let pad = (note & 0x0F) as usize;
        match note {
            // HOT CUE mode
            0x00..=0x07 => {
                if shift {
                    if pressed { Some(DjAction::DeckHotCueDelete { deck, pad_index: pad }) } else { None }
                } else {
                    Some(DjAction::DeckHotCue { deck, pad_index: pad, pressed })
                }
            }
            // BEAT JUMP mode
            0x20..=0x27 if pressed => Some(DjAction::DeckBeatJump { deck, beats: BEAT_JUMP_PADS[pad] }),
            // SAMPLER mode pads 1-3 are repurposed as stem mutes (Vocal / Drums / Bass), pad 4 = Slip
            0x30..=0x32 if pressed => Some(DjAction::DeckStemMuteToggle { deck, stem_index: pad }),
            0x33 if pressed => Some(DjAction::DeckSlipToggle { deck }),
            // BEAT LOOP mode
            0x60..=0x67 if pressed => Some(DjAction::DeckAutoLoop { deck, beats: BEAT_LOOP_PADS[pad] }),
            _ => None,
        }
    }

    fn deck_cc(deck: usize, cc: u8, value: u8) -> Option<DjAction> {
        match cc {
            0x00 => Some(DjAction::DeckTempoFader { deck, value: bipolar(value) }),
            0x04 => Some(DjAction::DeckTrim { deck, value: unipolar(value) }),
            0x07 => Some(DjAction::DeckEqHigh { deck, value: unipolar(value) }),
            0x0B => Some(DjAction::DeckEqMid { deck, value: unipolar(value) }),
            0x0F => Some(DjAction::DeckEqLow { deck, value: unipolar(value) }),
            0x13 => Some(DjAction::DeckVolumeFader { deck, value: unipolar(value) }),
            // Jog ring (pitch bend)
            0x21 => Some(DjAction::DeckJogRotate { deck, delta: relative(value) }),
            // Jog platter while touched (scratch)
            0x22 => Some(DjAction::DeckJogScratch { deck, delta: relative(value) }),
            // Shift + jog (search)
            0x23 | 0x26 => Some(DjAction::DeckJogSearch { deck, delta: relative(value) }),
            _ => None,
        }
    }

    fn mixer_cc(cc: u8, value: u8) -> Option<DjAction> {
        match cc {
            0x1F => Some(DjAction::Crossfader { value: bipolar(value) }),
            0x17 => Some(DjAction::DeckColorFx { deck: 0, value: bipolar(value) }),
            0x18 => Some(DjAction::DeckColorFx { deck: 1, value: bipolar(value) }),
            0x40 => {
                let delta = if value >= 64 { value as i32 - 128 } else { value as i32 };
                Some(DjAction::BrowseScroll { delta })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_transport_goes_to_the_right_deck() {
        assert_eq!(Flx4Parser::parse(&[0x90, 0x0B, 0x7F]), Some(DjAction::DeckPlayPause { deck: 0 }));
        assert_eq!(Flx4Parser::parse(&[0x91, 0x0B, 0x7F]), Some(DjAction::DeckPlayPause { deck: 1 }));
        assert_eq!(Flx4Parser::parse(&[0x91, 0x0C, 0x00]), Some(DjAction::DeckCue { deck: 1, pressed: false }));
    }

    #[test]
    fn pads_do_not_trigger_play() {
        // Pad 1 of deck 2 (hot cue mode) used to toggle PLAY on deck 1
        assert_eq!(
            Flx4Parser::parse(&[0x99, 0x00, 0x7F]),
            Some(DjAction::DeckHotCue { deck: 1, pad_index: 0, pressed: true })
        );
        assert_eq!(
            Flx4Parser::parse(&[0x97, 0x05, 0x7F]),
            Some(DjAction::DeckHotCue { deck: 0, pad_index: 5, pressed: true })
        );
        assert_eq!(
            Flx4Parser::parse(&[0x9A, 0x02, 0x7F]),
            Some(DjAction::DeckHotCueDelete { deck: 1, pad_index: 2 })
        );
    }

    #[test]
    fn mixer_section() {
        assert_eq!(Flx4Parser::parse(&[0xB6, 0x1F, 0x00]), Some(DjAction::Crossfader { value: -1.0 }));
        assert_eq!(Flx4Parser::parse(&[0xB6, 0x18, 0x7F]), Some(DjAction::DeckColorFx { deck: 1, value: 1.0 }));
        assert_eq!(Flx4Parser::parse(&[0xB6, 0x40, 0x7F]), Some(DjAction::BrowseScroll { delta: -1 }));
        assert_eq!(Flx4Parser::parse(&[0x96, 0x47, 0x7F]), Some(DjAction::LoadSelected { deck: 1 }));
    }

    #[test]
    fn jog_encoding() {
        assert_eq!(Flx4Parser::parse(&[0xB0, 0x22, 0x41]), Some(DjAction::DeckJogScratch { deck: 0, delta: 1.0 }));
        assert_eq!(Flx4Parser::parse(&[0xB1, 0x21, 0x3E]), Some(DjAction::DeckJogRotate { deck: 1, delta: -2.0 }));
    }
}
