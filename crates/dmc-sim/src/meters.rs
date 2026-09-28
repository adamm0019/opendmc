//! Style meter and Devil Trigger gauge. Thresholds are placeholders.

use serde::{Deserialize, Serialize};

pub const RANKS: [(&str, f32); 5] = [
    ("D", 0.0),
    ("C", 100.0),
    ("B", 250.0),
    ("A", 450.0),
    ("S", 700.0),
];
const STYLE_CAP: f32 = 1000.0;
/// Recent moves remembered for the variety penalty.
const HISTORY: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StyleMeter {
    pub points: f32,
    recent: Vec<String>,
    /// Ticks since points were last gained.
    pub idle_ticks: u32,
}

impl StyleMeter {
    /// Repeating one of the last few moves scores half.
    pub fn on_hit(&mut self, move_id: &str, base: f32) -> f32 {
        let repeat = self.recent.iter().any(|m| m == move_id);
        let gain = if repeat { base * 0.5 } else { base };
        self.points = (self.points + gain).min(STYLE_CAP);
        self.recent.push(move_id.to_string());
        if self.recent.len() > HISTORY {
            self.recent.remove(0);
        }
        self.idle_ticks = 0;
        gain
    }

    pub fn on_damaged(&mut self) {
        self.points = 0.0;
        self.recent.clear();
    }

    pub fn tick(&mut self, decay: f32) {
        self.idle_ticks += 1;
        if self.idle_ticks > 60 {
            self.points = (self.points - decay).max(0.0);
        }
    }

    pub fn rank(&self) -> Option<&'static str> {
        if self.points < RANKS[1].1 * 0.25 {
            return None;
        }
        RANKS
            .iter()
            .rev()
            .find(|(_, t)| self.points >= *t)
            .map(|(r, _)| *r)
    }
}

pub const DT_BAR: f32 = 1000.0;
const DT_MAX: f32 = 3.0 * DT_BAR;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DevilTrigger {
    pub gauge: f32,
    pub active: bool,
}

impl DevilTrigger {
    pub fn gain(&mut self, amount: f32) {
        if !self.active {
            self.gauge = (self.gauge + amount).min(DT_MAX);
        }
    }

    /// Toggle on a button press. Activation needs one full bar.
    pub fn toggle(&mut self) -> bool {
        if self.active {
            self.active = false;
            return true;
        }
        if self.gauge >= DT_BAR {
            self.active = true;
            return true;
        }
        false
    }

    /// Returns `true` on the tick the gauge runs dry.
    pub fn tick(&mut self, drain: f32) -> bool {
        if !self.active {
            return false;
        }
        self.gauge = (self.gauge - drain).max(0.0);
        if self.gauge == 0.0 {
            self.active = false;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variety_is_rewarded() {
        let mut s = StyleMeter::default();
        assert_eq!(s.on_hit("a", 20.0), 20.0);
        assert_eq!(s.on_hit("a", 20.0), 10.0);
        assert_eq!(s.on_hit("b", 20.0), 20.0);
    }

    #[test]
    fn ranks_climb_and_decay() {
        let mut s = StyleMeter::default();
        assert_eq!(s.rank(), None);
        s.points = 260.0;
        assert_eq!(s.rank(), Some("B"));
        for _ in 0..(60 + 100) {
            s.tick(1.0);
        }
        assert_eq!(s.points, 160.0);
        assert_eq!(s.rank(), Some("C"));
        s.on_damaged();
        assert_eq!(s.rank(), None);
    }

    #[test]
    fn devil_trigger_needs_a_bar_and_drains() {
        let mut dt = DevilTrigger::default();
        dt.gain(500.0);
        assert!(!dt.toggle());
        dt.gain(600.0);
        assert!(dt.toggle() && dt.active);
        dt.gain(1000.0);
        assert_eq!(dt.gauge, 1100.0, "no gain while active");
        let mut emptied = false;
        for _ in 0..1000 {
            emptied |= dt.tick(2.0);
        }
        assert!(emptied && !dt.active);
    }
}
