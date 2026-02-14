use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::time::{Duration, interval};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HopLatencyStats {
    pub sample_count: usize,
    pub p50_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

#[derive(Debug)]
struct AtomicSampleWindow {
    hop: &'static str,
    capacity: usize,
    write_index: AtomicUsize,
    sample_len: AtomicUsize,
    samples: Box<[AtomicU64]>,
}

impl AtomicSampleWindow {
    fn new(hop: &'static str, capacity: usize) -> Self {
        let mut samples = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            samples.push(AtomicU64::new(0));
        }

        Self {
            hop,
            capacity,
            write_index: AtomicUsize::new(0),
            sample_len: AtomicUsize::new(0),
            samples: samples.into_boxed_slice(),
        }
    }

    fn record(&self, duration_ns: u64) {
        let write = self.write_index.fetch_add(1, Ordering::Relaxed);
        let slot = modulo_index(write, self.capacity);
        if let Some(sample) = self.samples.get(slot) {
            sample.store(duration_ns, Ordering::Relaxed);
        }

        let _update = self
            .sample_len
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                if value < self.capacity {
                    Some(value.saturating_add(1))
                } else {
                    None
                }
            });
    }

    fn snapshot_stats(&self) -> Option<(&'static str, HopLatencyStats)> {
        let len = self.sample_len.load(Ordering::Acquire).min(self.capacity);
        if len == 0 {
            return None;
        }

        let write = self.write_index.load(Ordering::Acquire);
        let start = write.saturating_sub(len);

        let mut values = Vec::with_capacity(len);
        for offset in 0..len {
            let index = modulo_index(start.saturating_add(offset), self.capacity);
            let value = self
                .samples
                .get(index)
                .map_or(0, |sample| sample.load(Ordering::Relaxed));
            values.push(value);
        }

        Some((self.hop, stats_from_samples(&values)))
    }
}

#[derive(Debug)]
pub struct LatencyTelemetry {
    enabled: bool,
    slo_threshold_ns: u64,
    ingress_to_engine: AtomicSampleWindow,
    engine_classification: AtomicSampleWindow,
    strategy_dispatch: AtomicSampleWindow,
    dropped_unknown_hops: AtomicU64,
}

impl LatencyTelemetry {
    pub fn new(sample_capacity: usize, slo_threshold_ns: u64) -> Self {
        Self::with_mode(true, sample_capacity, slo_threshold_ns)
    }

    pub fn disabled() -> Self {
        Self::with_mode(false, 1, 0)
    }

    fn with_mode(enabled: bool, sample_capacity: usize, slo_threshold_ns: u64) -> Self {
        let capacity = sample_capacity.max(1);
        Self {
            enabled,
            slo_threshold_ns,
            ingress_to_engine: AtomicSampleWindow::new("ingress_to_engine_ns", capacity),
            engine_classification: AtomicSampleWindow::new("engine_classification_ns", capacity),
            strategy_dispatch: AtomicSampleWindow::new("strategy_dispatch_ns", capacity),
            dropped_unknown_hops: AtomicU64::new(0),
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn record(&self, hop: &'static str, duration_ns: u64) {
        if !self.enabled {
            return;
        }

        match hop {
            "ingress_to_engine_ns" => self.ingress_to_engine.record(duration_ns),
            "engine_classification_ns" => self.engine_classification.record(duration_ns),
            "strategy_dispatch_ns" => self.strategy_dispatch.record(duration_ns),
            _ => {
                self.dropped_unknown_hops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn snapshot_all(&self) -> Vec<(&'static str, HopLatencyStats)> {
        if !self.enabled {
            return Vec::new();
        }

        let mut stats = Vec::with_capacity(3);

        if let Some(value) = self.ingress_to_engine.snapshot_stats() {
            stats.push(value);
        }
        if let Some(value) = self.engine_classification.snapshot_stats() {
            stats.push(value);
        }
        if let Some(value) = self.strategy_dispatch.snapshot_stats() {
            stats.push(value);
        }

        stats.sort_by(|left, right| left.0.cmp(right.0));
        stats
    }

    pub fn spawn_reporter(self: std::sync::Arc<Self>, period: Duration) {
        if !self.enabled {
            return;
        }

        tokio::spawn(async move {
            let mut ticker = interval(period);
            loop {
                ticker.tick().await;
                self.emit_periodic_report();
            }
        });
    }

    fn emit_periodic_report(&self) {
        let stats = self.snapshot_all();
        for (hop, hop_stats) in stats {
            log::info!(
                "Latency telemetry > hop={} count={} p50={}ns p99={}ns max={}ns",
                hop,
                hop_stats.sample_count,
                hop_stats.p50_ns,
                hop_stats.p99_ns,
                hop_stats.max_ns
            );

            if hop_stats.p99_ns > self.slo_threshold_ns || hop_stats.max_ns > self.slo_threshold_ns
            {
                log::warn!(
                    "Latency SLO alert > hop={} threshold={}ns p99={}ns max={}ns",
                    hop,
                    self.slo_threshold_ns,
                    hop_stats.p99_ns,
                    hop_stats.max_ns
                );
            }
        }

        let dropped_unknown_hops = self.dropped_unknown_hops.load(Ordering::Relaxed);
        if dropped_unknown_hops > 0 {
            log::warn!(
                "Latency telemetry > dropped unsupported hop samples={}",
                dropped_unknown_hops
            );
        }
    }
}

fn modulo_index(value: usize, modulus: usize) -> usize {
    value.checked_rem(modulus).unwrap_or(0)
}

fn stats_from_samples(samples: &[u64]) -> HopLatencyStats {
    if samples.is_empty() {
        return HopLatencyStats {
            sample_count: 0,
            p50_ns: 0,
            p99_ns: 0,
            max_ns: 0,
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();

    let p50_ns = percentile_bps(&sorted, 5_000);
    let p99_ns = percentile_bps(&sorted, 9_900);
    let max_ns = sorted.last().copied().unwrap_or(0);

    HopLatencyStats {
        sample_count: sorted.len(),
        p50_ns,
        p99_ns,
        max_ns,
    }
}

fn percentile_bps(sorted_samples: &[u64], bps: u16) -> u64 {
    if sorted_samples.is_empty() {
        return 0;
    }

    let max_index = sorted_samples.len().saturating_sub(1);
    let max_index_u64 = u64::try_from(max_index).unwrap_or(u64::MAX);
    let numerator = u128::from(max_index_u64).saturating_mul(u128::from(bps));
    let index_u128 = numerator / 10_000_u128;
    let index = usize::try_from(index_u128).unwrap_or(max_index);

    sorted_samples
        .get(index)
        .copied()
        .or_else(|| sorted_samples.get(max_index).copied())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::LatencyTelemetry;

    #[test]
    fn computes_p50_p99_and_max() {
        let telemetry = LatencyTelemetry::new(64, 1_000_000);
        for value in [10_u64, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
            telemetry.record("ingress_to_engine_ns", value);
        }

        let snapshots = telemetry.snapshot_all();
        assert_eq!(snapshots.len(), 1);
        assert!(!snapshots.is_empty());

        if let Some((_, stats)) = snapshots.first().copied() {
            assert_eq!(stats.sample_count, 10);
            assert_eq!(stats.p50_ns, 50);
            assert_eq!(stats.p99_ns, 90);
            assert_eq!(stats.max_ns, 100);
        }
    }

    #[test]
    fn keeps_only_recent_samples_per_hop() {
        let telemetry = LatencyTelemetry::new(3, 1_000_000);
        telemetry.record("ingress_to_engine_ns", 1);
        telemetry.record("ingress_to_engine_ns", 2);
        telemetry.record("ingress_to_engine_ns", 3);
        telemetry.record("ingress_to_engine_ns", 4);

        let snapshots = telemetry.snapshot_all();
        assert_eq!(snapshots.len(), 1);
        assert!(!snapshots.is_empty());
        if let Some((_, stats)) = snapshots.first().copied() {
            assert_eq!(stats.sample_count, 3);
            assert_eq!(stats.max_ns, 4);
        }
    }

    #[test]
    fn disabled_telemetry_is_noop() {
        let telemetry = LatencyTelemetry::disabled();
        telemetry.record("ingress_to_engine_ns", 1_000);
        telemetry.record("engine_classification_ns", 2_000);

        let snapshots = telemetry.snapshot_all();
        assert!(snapshots.is_empty());
        assert!(!telemetry.is_enabled());
    }
}
