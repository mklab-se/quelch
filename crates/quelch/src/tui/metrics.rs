//! Primitive metrics used by the TUI: a 60s ring buffer and Azure counters.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Rolling 60-entry (~60s) ring buffer keyed by second.
#[derive(Debug, Default)]
pub struct Throughput {
    buckets: VecDeque<(Instant, u64)>,
}

impl Throughput {
    /// Add `n` items observed now.
    pub fn add(&mut self, now: Instant, n: u64) {
        self.prune(now);
        if let Some(last) = self.buckets.back_mut()
            && now.duration_since(last.0) < Duration::from_secs(1)
        {
            last.1 += n;
            return;
        }
        self.buckets.push_back((now, n));
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&(t, _)) = self.buckets.front() {
            if now.duration_since(t) > Duration::from_secs(60) {
                self.buckets.pop_front();
            } else {
                break;
            }
        }
    }

    /// Sum across the last 60s.
    pub fn per_minute(&mut self, now: Instant) -> u64 {
        self.prune(now);
        self.buckets.iter().map(|(_, n)| *n).sum()
    }

    /// Return up to 60 per-second samples for sparkline rendering, oldest first.
    pub fn samples(&self) -> Vec<u64> {
        self.buckets.iter().map(|(_, n)| *n).collect()
    }

    /// Return `(bucket_index, count)` points suitable for `ratatui::Chart`.
    pub fn chart_points(&self) -> Vec<(f64, f64)> {
        self.buckets
            .iter()
            .enumerate()
            .map(|(i, (_, n))| (i as f64, *n as f64))
            .collect()
    }
}

/// Azure panel state: rolling throughput + latency window + response counters.
#[derive(Debug, Default)]
pub struct AzurePanel {
    pub requests_per_sec: Throughput,
    pub errors_5xx_per_sec: Throughput,
    pub latency_samples: VecDeque<Duration>,
    pub total: u64,
    pub count_4xx: u64,
    pub count_5xx: u64,
    pub count_throttled: u64,
}

const LATENCY_CAP: usize = 5000;

impl AzurePanel {
    pub fn on_response(&mut self, at: Instant, status: u16, latency: Duration, throttled: bool) {
        self.total += 1;
        self.requests_per_sec.add(at, 1);
        if status >= 500 {
            self.count_5xx += 1;
            self.errors_5xx_per_sec.add(at, 1);
        } else if status >= 400 {
            self.count_4xx += 1;
        }
        if throttled {
            self.count_throttled += 1;
        }
        if self.latency_samples.len() >= LATENCY_CAP {
            self.latency_samples.pop_front();
        }
        self.latency_samples.push_back(latency);
    }

    pub fn p50_p95(&self) -> (Duration, Duration) {
        if self.latency_samples.is_empty() {
            return (Duration::ZERO, Duration::ZERO);
        }
        let mut v: Vec<Duration> = self.latency_samples.iter().copied().collect();
        v.sort();
        let p50 = v[v.len() / 2];
        let p95 = v[(v.len() * 95 / 100).min(v.len() - 1)];
        (p50, p95)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throughput_accumulates_then_expires() {
        let mut t = Throughput::default();
        let t0 = Instant::now();
        t.add(t0, 5);
        t.add(t0 + Duration::from_millis(100), 3);
        assert_eq!(t.per_minute(t0 + Duration::from_secs(1)), 8);
        assert_eq!(t.per_minute(t0 + Duration::from_secs(120)), 0);
    }

    #[test]
    fn azure_panel_p50_p95() {
        let mut a = AzurePanel::default();
        for ms in 10..=110 {
            a.on_response(Instant::now(), 200, Duration::from_millis(ms), false);
        }
        let (p50, p95) = a.p50_p95();
        assert!(p50.as_millis() >= 55 && p50.as_millis() <= 65);
        assert!(p95.as_millis() >= 100);
    }

    #[test]
    fn azure_panel_counts_by_status() {
        let mut a = AzurePanel::default();
        a.on_response(Instant::now(), 200, Duration::from_millis(10), false);
        a.on_response(Instant::now(), 404, Duration::from_millis(10), false);
        a.on_response(Instant::now(), 500, Duration::from_millis(10), false);
        a.on_response(Instant::now(), 429, Duration::from_millis(10), true);
        assert_eq!(a.total, 4);
        assert_eq!(a.count_4xx, 2);
        assert_eq!(a.count_5xx, 1);
        assert_eq!(a.count_throttled, 1);
    }

    #[test]
    fn chart_points_returns_ordered_xy_pairs() {
        let mut t = Throughput::default();
        let t0 = Instant::now();
        t.add(t0, 3);
        t.add(t0 + Duration::from_secs(2), 5);
        let pts = t.chart_points();
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].1, 3.0);
        assert_eq!(pts[1].1, 5.0);
        assert!(pts[0].0 < pts[1].0);
    }
}
