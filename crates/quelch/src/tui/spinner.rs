//! Tiny frame-based spinner that cycles through four glyphs.

#[derive(Debug, Default)]
pub struct Spinner {
    tick: u32,
}

impl Spinner {
    const FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];

    /// Call once per redraw tick. At 5 Hz, glyph changes every two ticks
    /// — a full rotation every ~1.6 seconds.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn glyph(&self) -> char {
        Self::FRAMES[(self.tick as usize / 2) % 4]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_all_frames() {
        let mut s = Spinner::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            seen.insert(s.glyph());
            s.tick();
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn glyph_changes_every_two_ticks() {
        let mut s = Spinner::default();
        let a = s.glyph();
        s.tick();
        assert_eq!(s.glyph(), a);
        s.tick();
        assert_ne!(s.glyph(), a);
    }
}
