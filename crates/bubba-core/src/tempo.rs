//! Tempo fader -> playback rate.
//!
//! The fader maps linearly around an anchor, in steps of the 0 % tempo:
//!     rate = ref_rate + (fader - anchor) * range * zero_rate
//! With `anchor = 0, ref_rate = zero_rate` this is the classic absolute fader;
//! re-anchoring at the current position gives Serato-style relative pitch
//! without any jump. A pending `pickup` is rekordbox / Pioneer soft takeover:
//! the fader is ignored until it reaches the position matching the current tempo.

/// Fader travel that counts as "on the pickup point" (0.5 % of the -1..+1 range)
const PICKUP_TOLERANCE: f32 = 0.01;
const MIN_RATE: f64 = 0.05;

#[derive(Debug, Clone)]
pub struct TempoControl {
    /// Playback speed relative to the track's native tempo (1.0 = original speed)
    pub rate: f64,
    /// Rate displayed as 0 % (moved by Serato-style zero point reassignment)
    pub zero_rate: f64,
    ref_rate: f64,
    anchor: f32,
    /// Last tempo fader position (-1.0 .. +1.0)
    pub fader: f32,
    fader_known: bool,
    /// Fader range (0.08 = ±8 %)
    pub range: f32,
    /// Soft takeover target and the side of it the fader was on
    pickup: Option<(f32, f32)>,
}

impl TempoControl {
    pub fn new() -> Self {
        TempoControl {
            rate: 1.0,
            zero_rate: 1.0,
            ref_rate: 1.0,
            anchor: 0.0,
            fader: 0.0,
            fader_known: false,
            range: 0.08,
            pickup: None,
        }
    }

    /// New track: absolute fader around the original tempo
    pub fn reset_absolute(&mut self) {
        self.zero_rate = 1.0;
        self.ref_rate = 1.0;
        self.anchor = 0.0;
        self.pickup = None;
        self.rate = self.mapped(self.fader);
    }

    fn mapped(&self, fader: f32) -> f64 {
        (self.ref_rate + (fader - self.anchor) as f64 * self.range as f64 * self.zero_rate).max(MIN_RATE)
    }

    /// Pitch relative to the 0 % point, in percent
    pub fn percent(&self) -> f64 {
        (self.rate / self.zero_rate - 1.0) * 100.0
    }

    /// Fader position the user has to reach before the fader takes over again
    pub fn pickup_position(&self) -> Option<f32> {
        self.pickup.map(|p| p.0)
    }

    /// Fader moved on a deck that controls its own tempo
    pub fn move_fader(&mut self, value: f32) {
        let value = value.clamp(-1.0, 1.0);
        self.fader_known = true;
        if let Some((target, side)) = self.pickup {
            let reached = (value - target).abs() <= PICKUP_TOLERANCE || (value - target).signum() != side;
            if !reached {
                self.fader = value;
                return;
            }
            self.pickup = None;
        }
        self.fader = value;
        self.rate = self.mapped(value);
    }

    /// Fader moved while the tempo is owned by someone else (rekordbox sync): remember only
    pub fn track_fader(&mut self, value: f32) {
        self.fader_known = true;
        self.fader = value.clamp(-1.0, 1.0);
    }

    /// Fader moved in relative mode: returns the new rate, measured from the current tempo
    pub fn move_fader_relative(&mut self, value: f32) -> f64 {
        let value = value.clamp(-1.0, 1.0);
        if !self.fader_known {
            self.track_fader(value);
            self.reanchor();
            return self.rate;
        }
        self.fader = value;
        self.pickup = None;
        self.rate = self.mapped(value);
        self.rate
    }

    /// Sets the speed and keeps the fader working relative to it (no jump on the next move)
    pub fn set_rate_relative(&mut self, rate: f64) {
        self.rate = rate.max(MIN_RATE);
        self.reanchor();
    }

    fn reanchor(&mut self) {
        self.ref_rate = self.rate;
        self.anchor = self.fader;
    }

    /// Serato zero point reassignment: the current tempo becomes the new 0 %, speed unchanged
    pub fn rezero(&mut self) {
        self.zero_rate = self.rate;
        self.pickup = None;
        self.reanchor();
    }

    /// Back to an absolute fader around the current 0 %, with soft takeover so the
    /// tempo does not jump when the fader is not where the tempo is.
    pub fn begin_pickup(&mut self) {
        self.ref_rate = self.zero_rate;
        self.anchor = 0.0;
        let target = (((self.rate / self.zero_rate) - 1.0) / self.range as f64) as f32;
        let target = target.clamp(-1.0, 1.0);
        if (self.fader - target).abs() <= PICKUP_TOLERANCE {
            self.pickup = None;
        } else {
            self.pickup = Some((target, (self.fader - target).signum()));
        }
    }

    pub fn set_range(&mut self, range: f32) {
        self.range = range.clamp(0.02, 1.0);
        if self.pickup.is_some() {
            self.begin_pickup();
        } else {
            // Keep the current tempo: re-anchor the fader
            self.reanchor();
        }
    }
}

impl Default for TempoControl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_fader() {
        let mut t = TempoControl::new();
        t.move_fader(0.5);
        // range is stored as f32
        assert!((t.rate - 1.04).abs() < 1e-6);
        assert!((t.percent() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn pickup_waits_for_the_fader() {
        let mut t = TempoControl::new();
        t.move_fader(0.0);
        // Sync moved the tempo to +2 % while the fader stayed at 0
        t.rate = 1.02;
        t.begin_pickup();
        assert_eq!(t.pickup_position(), Some(0.25));
        t.move_fader(0.1);
        assert!((t.rate - 1.02).abs() < 1e-6, "no jump before the pickup point");
        t.move_fader(0.3); // crossed 0.25
        assert!(t.pickup_position().is_none());
        assert!((t.rate - 1.024).abs() < 1e-6);
    }

    #[test]
    fn relative_after_rezero() {
        let mut t = TempoControl::new();
        t.move_fader(-0.4);
        t.set_rate_relative(1.1);
        t.rezero();
        assert!(t.percent().abs() < 1e-9);
        let r = t.move_fader_relative(0.1); // +0.5 of travel at ±8 % = +4 % of 1.1
        assert!((r - 1.1 * 1.04).abs() < 1e-6);
    }
}
