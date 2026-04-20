//! SimOpts — parameters controlling the simulator.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SimOpts {
    pub duration: Option<Duration>,
    pub seed: Option<u64>,
    pub rate_multiplier: f64,
    pub fault_rate: f64,
    pub assert_docs: Option<u64>,
    pub mock_port: Option<u16>,
    pub snapshot_to: Option<std::path::PathBuf>,
    pub snapshot_frames: u32,
    pub snapshot_width: u16,
    pub snapshot_height: u16,
}

impl Default for SimOpts {
    fn default() -> Self {
        Self {
            duration: None,
            seed: None,
            rate_multiplier: 1.0,
            fault_rate: 0.03,
            assert_docs: None,
            mock_port: None,
            snapshot_to: None,
            snapshot_frames: 10,
            snapshot_width: 120,
            snapshot_height: 40,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let o = SimOpts::default();
        assert_eq!(o.rate_multiplier, 1.0);
        assert!((o.fault_rate - 0.03).abs() < 1e-9);
        assert!(o.duration.is_none());
    }
}
