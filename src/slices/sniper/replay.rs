use std::time::Instant;

use solana_sdk::{message::compiled_instruction::CompiledInstruction, pubkey::Pubkey};

use crate::adapters::raydium::{
    RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_INITIALIZE2_TAG, RAYDIUM_V4_PROGRAM_ID,
    RAYDIUM_V4_SWAP_BASE_IN_TAG, STANDARD_AMM_INITIALIZE, STANDARD_AMM_SWAP_BASE_INPUT,
    classify_raydium_creation_instructions,
};

const MIN_EVENTS_PER_PATH: usize = 1_000_000;

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
    pub scan_repeats: usize,
    pub sof_creation_path: ReplayPathStats,
    pub sof_swap_path: ReplayPathStats,
}

pub fn run_synthetic_replay(event_count: usize, burst_size: usize) -> ReplayBenchmarkReport {
    let total_events = event_count.max(1);
    let burst = burst_size.max(1);
    let scan_repeats = repeats_for(total_events);
    let structured_creation_events =
        build_structured_dataset(total_events, ReplayWorkload::PoolCreation);
    let structured_swap_events = build_structured_dataset(total_events, ReplayWorkload::SwapFlow);
    let sof_creation_path = benchmark_structured_path(
        "sof_structured_creation_scan",
        &structured_creation_events,
        burst,
        scan_repeats,
    );
    let sof_swap_path = benchmark_structured_path(
        "sof_structured_swap_scan",
        &structured_swap_events,
        burst,
        scan_repeats,
    );

    ReplayBenchmarkReport {
        event_count: total_events,
        burst_size: burst,
        scan_repeats,
        sof_creation_path,
        sof_swap_path,
    }
}

pub fn log_replay_report(report: &ReplayBenchmarkReport) {
    log::info!(
        "Replay benchmark > events={} burst={} repeats={}",
        report.event_count,
        report.burst_size,
        report.scan_repeats
    );

    for path in [&report.sof_creation_path, &report.sof_swap_path] {
        log::info!(
            "Replay benchmark > path={} candidates={} throughput={}ev/s p50={}ns p99={}ns max={}ns",
            path.path,
            path.candidate_events,
            path.throughput_events_per_sec,
            path.p50_ns,
            path.p99_ns,
            path.max_ns
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayWorkload {
    PoolCreation,
    SwapFlow,
}

#[derive(Clone, Debug)]
struct StructuredSyntheticEvent {
    account_keys: Vec<Pubkey>,
    instructions: Vec<CompiledInstruction>,
}

fn repeats_for(total_events: usize) -> usize {
    let bounded_total_events = total_events.max(1);
    MIN_EVENTS_PER_PATH.div_ceil(bounded_total_events)
}

fn build_structured_dataset(
    total_events: usize,
    workload: ReplayWorkload,
) -> Vec<StructuredSyntheticEvent> {
    let cpmm_program = Pubkey::from_str_const(RAYDIUM_STANDARD_AMM_PROGRAM_ID);
    let openbook_program = Pubkey::from_str_const(RAYDIUM_V4_PROGRAM_ID);
    let filler_accounts = [
        Pubkey::new_from_array([1_u8; 32]),
        Pubkey::new_from_array([2_u8; 32]),
        Pubkey::new_from_array([3_u8; 32]),
    ];

    let mut dataset = Vec::with_capacity(total_events);
    for index in 0..total_events {
        let mut account_keys = Vec::with_capacity(4);
        let instructions = if index.is_multiple_of(2) {
            account_keys.push(cpmm_program);
            vec![CompiledInstruction::new_from_raw_parts(
                0,
                structured_instruction_data(workload, false),
                vec![],
            )]
        } else {
            account_keys.push(openbook_program);
            vec![CompiledInstruction::new_from_raw_parts(
                0,
                structured_instruction_data(workload, true),
                vec![],
            )]
        };
        account_keys.extend_from_slice(&filler_accounts);
        dataset.push(StructuredSyntheticEvent {
            account_keys,
            instructions,
        });
    }
    dataset
}

fn structured_instruction_data(workload: ReplayWorkload, is_openbook: bool) -> Vec<u8> {
    match (workload, is_openbook) {
        (ReplayWorkload::PoolCreation, true) => vec![RAYDIUM_V4_INITIALIZE2_TAG],
        (ReplayWorkload::PoolCreation, false) => STANDARD_AMM_INITIALIZE.to_vec(),
        (ReplayWorkload::SwapFlow, true) => vec![RAYDIUM_V4_SWAP_BASE_IN_TAG],
        (ReplayWorkload::SwapFlow, false) => STANDARD_AMM_SWAP_BASE_INPUT.to_vec(),
    }
}

fn benchmark_structured_path(
    path: &'static str,
    events: &[StructuredSyntheticEvent],
    burst_size: usize,
    repeats: usize,
) -> ReplayPathStats {
    let cpmm_program = Pubkey::from_str_const(RAYDIUM_STANDARD_AMM_PROGRAM_ID);
    let openbook_program = Pubkey::from_str_const(RAYDIUM_V4_PROGRAM_ID);
    let started_at = Instant::now();
    let mut candidate_count = 0_usize;
    let mut per_event_ns = Vec::with_capacity(events.len().saturating_mul(repeats));

    for _ in 0..repeats {
        for chunk in events.chunks(burst_size) {
            for synthetic in chunk {
                let event_start = Instant::now();
                if classify_raydium_creation_instructions(
                    &synthetic.account_keys,
                    &synthetic.instructions,
                    cpmm_program,
                    openbook_program,
                )
                .is_some()
                {
                    candidate_count = candidate_count.saturating_add(1);
                }
                per_event_ns.push(elapsed_ns_u64(event_start.elapsed()));
            }
        }
    }

    build_replay_path_stats(
        path,
        events.len().saturating_mul(repeats),
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
        assert!(report.scan_repeats > 0);
        assert_eq!(report.sof_creation_path.total_events, 1_000_192);
        assert_eq!(report.sof_swap_path.total_events, 1_000_192);
        assert!(report.sof_creation_path.candidate_events > 0);
        assert_eq!(report.sof_swap_path.candidate_events, 0);
    }
}
