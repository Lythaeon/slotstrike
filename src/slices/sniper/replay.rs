use std::time::Instant;

use crate::{
    adapters::raydium::{RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID},
    domain::events::{
        IngressMetadata, IngressSource, RawLogEvent, normalize_hardware_timestamp_ns,
        unix_timestamp_now_ns,
    },
};

use super::pool_filter::{is_pool_creation_candidate_logs, is_pool_creation_dma_payload};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayPathStats {
    pub path: &'static str,
    pub total_events: usize,
    pub candidate_events: usize,
    pub elapsed_ns: u64,
    pub throughput_events_per_sec: u64,
    pub p50_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayBenchmarkReport {
    pub event_count: usize,
    pub burst_size: usize,
    pub fpga_path: ReplayPathStats,
    pub kernel_bypass_path: ReplayPathStats,
}

pub fn run_synthetic_replay(event_count: usize, burst_size: usize) -> ReplayBenchmarkReport {
    let total_events = event_count.max(1);
    let burst = burst_size.max(1);
    let synthetic = build_synthetic_dataset(total_events);

    let fpga_path = benchmark_fpga_path(&synthetic, burst);
    let kernel_bypass_path = benchmark_kernel_path(&synthetic, burst);

    ReplayBenchmarkReport {
        event_count: total_events,
        burst_size: burst,
        fpga_path,
        kernel_bypass_path,
    }
}

pub fn log_replay_report(report: &ReplayBenchmarkReport) {
    log::info!(
        "Replay benchmark > events={} burst={}",
        report.event_count,
        report.burst_size
    );
    log::info!(
        "Replay benchmark > path={} candidates={} throughput={}ev/s p50={}ns p99={}ns max={}ns",
        report.fpga_path.path,
        report.fpga_path.candidate_events,
        report.fpga_path.throughput_events_per_sec,
        report.fpga_path.p50_ns,
        report.fpga_path.p99_ns,
        report.fpga_path.max_ns
    );
    log::info!(
        "Replay benchmark > path={} candidates={} throughput={}ev/s p50={}ns p99={}ns max={}ns",
        report.kernel_bypass_path.path,
        report.kernel_bypass_path.candidate_events,
        report.kernel_bypass_path.throughput_events_per_sec,
        report.kernel_bypass_path.p50_ns,
        report.kernel_bypass_path.p99_ns,
        report.kernel_bypass_path.max_ns
    );
}

#[derive(Clone, Debug)]
struct SyntheticEvent {
    kernel_event: RawLogEvent,
    dma_payload: Vec<u8>,
}

fn build_synthetic_dataset(total_events: usize) -> Vec<SyntheticEvent> {
    let mut dataset = Vec::with_capacity(total_events);
    for index in 0..total_events {
        let signature = format!("synthetic_sig_{index}");
        let logs = synthetic_logs(index);
        let payload = synthetic_dma_payload(&signature, &logs);
        let receive_timestamp_ns = unix_timestamp_now_ns();
        let normalized_timestamp_ns = normalize_hardware_timestamp_ns(None, receive_timestamp_ns);

        dataset.push(SyntheticEvent {
            kernel_event: RawLogEvent {
                signature,
                logs,
                has_error: false,
                ingress: IngressMetadata {
                    source: IngressSource::KernelBypass,
                    hardware_timestamp_ns: None,
                    received_timestamp_ns: receive_timestamp_ns,
                    normalized_timestamp_ns,
                },
            },
            dma_payload: payload,
        });
    }
    dataset
}

fn synthetic_logs(index: usize) -> Vec<String> {
    let is_openbook = index.is_multiple_of(2);
    if is_openbook {
        vec![
            format!("Program {} invoke [1]", RAYDIUM_V4_PROGRAM_ID),
            "Program log: initialize2".to_owned(),
        ]
    } else {
        vec![
            format!("Program {} invoke [1]", RAYDIUM_STANDARD_AMM_PROGRAM_ID),
            "Program log: vault_0_amount:1234, vault_1_amount:5678".to_owned(),
        ]
    }
}

fn synthetic_dma_payload(signature: &str, logs: &[String]) -> Vec<u8> {
    let mut payload = format!("signature={signature}\nhas_error=0\n");
    for log_line in logs {
        payload.push_str("log=");
        payload.push_str(log_line);
        payload.push('\n');
    }
    payload.into_bytes()
}

fn benchmark_fpga_path(events: &[SyntheticEvent], burst_size: usize) -> ReplayPathStats {
    let started_at = Instant::now();
    let mut candidate_count = 0_usize;
    let mut per_event_ns = Vec::with_capacity(events.len());

    for chunk in events.chunks(burst_size) {
        for synthetic in chunk {
            let event_start = Instant::now();
            if is_pool_creation_dma_payload(&synthetic.dma_payload) {
                candidate_count = candidate_count.saturating_add(1);
            }
            per_event_ns.push(elapsed_ns_u64(event_start.elapsed()));
        }
    }

    build_replay_path_stats(
        "fpga_dma",
        events.len(),
        candidate_count,
        elapsed_ns_u64(started_at.elapsed()),
        &per_event_ns,
    )
}

fn benchmark_kernel_path(events: &[SyntheticEvent], burst_size: usize) -> ReplayPathStats {
    let started_at = Instant::now();
    let mut candidate_count = 0_usize;
    let mut per_event_ns = Vec::with_capacity(events.len());

    for chunk in events.chunks(burst_size) {
        for synthetic in chunk {
            let event_start = Instant::now();
            if is_pool_creation_candidate_logs(&synthetic.kernel_event.logs) {
                candidate_count = candidate_count.saturating_add(1);
            }
            per_event_ns.push(elapsed_ns_u64(event_start.elapsed()));
        }
    }

    build_replay_path_stats(
        "kernel_bypass",
        events.len(),
        candidate_count,
        elapsed_ns_u64(started_at.elapsed()),
        &per_event_ns,
    )
}

fn build_replay_path_stats(
    path: &'static str,
    total_events: usize,
    candidate_events: usize,
    elapsed_ns: u64,
    per_event_ns: &[u64],
) -> ReplayPathStats {
    let mut sorted = per_event_ns.to_vec();
    sorted.sort_unstable();

    let p50_ns = percentile_bps(&sorted, 5_000);
    let p99_ns = percentile_bps(&sorted, 9_900);
    let max_ns = sorted.last().copied().unwrap_or(0);
    let throughput_events_per_sec = throughput_per_second(total_events, elapsed_ns);

    ReplayPathStats {
        path,
        total_events,
        candidate_events,
        elapsed_ns,
        throughput_events_per_sec,
        p50_ns,
        p99_ns,
        max_ns,
    }
}

fn throughput_per_second(total_events: usize, elapsed_ns: u64) -> u64 {
    if elapsed_ns == 0 {
        return 0;
    }

    let total_events_u64 = u64::try_from(total_events).unwrap_or(u64::MAX);
    let numerator = u128::from(total_events_u64).saturating_mul(1_000_000_000_u128);
    let value = numerator.checked_div(u128::from(elapsed_ns)).unwrap_or(0);
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn percentile_bps(sorted: &[u64], bps: u16) -> u64 {
    if sorted.is_empty() {
        return 0;
    }

    let max_index = sorted.len().saturating_sub(1);
    let max_index_u64 = u64::try_from(max_index).unwrap_or(u64::MAX);
    let numerator = u128::from(max_index_u64).saturating_mul(u128::from(bps));
    let index_u128 = numerator / 10_000_u128;
    let index = usize::try_from(index_u128).unwrap_or(max_index);
    sorted
        .get(index)
        .copied()
        .or_else(|| sorted.get(max_index).copied())
        .unwrap_or(0)
}

fn elapsed_ns_u64(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::run_synthetic_replay;

    #[test]
    fn produces_non_empty_reports() {
        let report = run_synthetic_replay(256, 32);

        assert_eq!(report.event_count, 256);
        assert_eq!(report.burst_size, 32);
        assert_eq!(report.fpga_path.total_events, 256);
        assert_eq!(report.kernel_bypass_path.total_events, 256);
        assert!(report.fpga_path.candidate_events > 0);
        assert!(report.kernel_bypass_path.candidate_events > 0);
    }
}
