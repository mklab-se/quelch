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
