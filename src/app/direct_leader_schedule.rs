use std::{
    collections::BTreeMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use sof::framework::{
    ControlPlaneSource, LeaderScheduleEntry, LeaderScheduleEvent, ObserverPlugin,
};
use sof_tx::{
    adapters::PluginHostTxProviderAdapter,
    submit::{TxFlowSafetyIssue, TxFlowSafetyQuality, TxFlowSafetySnapshot, TxFlowSafetySource},
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_epoch_schedule::EpochSchedule;
use solana_sdk::pubkey::Pubkey;
use tokio::{task::JoinHandle, time::MissedTickBehavior};

const DIRECT_SCHEDULE_CURSOR_TICK: Duration = Duration::from_millis(250);
const DIRECT_SCHEDULE_RPC_RETRY: Duration = Duration::from_secs(5);
const MIN_LEADER_WINDOW_SLOTS: usize = 512;
const MAX_LEADER_WINDOW_SLOTS: usize = 2_048;
const LEADER_WINDOW_MULTIPLIER: usize = 256;
const MAX_RECENT_BLOCKHASH_SLOT_LAG: u64 = 32;
const MAX_CLUSTER_TOPOLOGY_SLOT_LAG: u64 = 64;
const MAX_LEADER_SCHEDULE_SLOT_LAG: u64 = 128;
const MAX_CONTROL_PLANE_SLOT_SPREAD: u64 = 32;

#[derive(Clone)]
pub struct DirectLeaderScheduleSafetySource {
    adapter: Arc<PluginHostTxProviderAdapter>,
}

impl DirectLeaderScheduleSafetySource {
    #[must_use]
    pub const fn new(adapter: Arc<PluginHostTxProviderAdapter>) -> Self {
        Self { adapter }
    }
}

impl TxFlowSafetySource for DirectLeaderScheduleSafetySource {
    fn toxic_flow_snapshot(&self) -> TxFlowSafetySnapshot {
        let snapshot = self.adapter.control_plane_snapshot();
        let effective_tip = snapshot
            .latest_recent_blockhash_slot
            .or(snapshot.leader_schedule_slot)
            .or(snapshot.cluster_topology_slot);

        let mut missing_control_plane = false;
        let mut stale_control_plane = false;
        let mut degraded_control_plane = false;

        if snapshot.latest_recent_blockhash_slot.is_none()
            || snapshot.cluster_topology_slot.is_none()
            || snapshot.leader_schedule_slot.is_none()
            || snapshot.known_leader_target_addrs == 0
        {
            missing_control_plane = true;
        }

        if let Some(tip_slot) = effective_tip {
            stale_control_plane |= is_slot_lag_stale(
                snapshot.latest_recent_blockhash_slot,
                tip_slot,
                MAX_RECENT_BLOCKHASH_SLOT_LAG,
            );
            stale_control_plane |= is_slot_lag_stale(
                snapshot.cluster_topology_slot,
                tip_slot,
                MAX_CLUSTER_TOPOLOGY_SLOT_LAG,
            );
            stale_control_plane |= is_slot_lag_stale(
                snapshot.leader_schedule_slot,
                tip_slot,
                MAX_LEADER_SCHEDULE_SLOT_LAG,
            );
            degraded_control_plane |=
                control_plane_slot_spread(snapshot, tip_slot) > MAX_CONTROL_PLANE_SLOT_SPREAD;
        } else {
            missing_control_plane = true;
        }

        let (quality, issues) = if missing_control_plane {
            (
                TxFlowSafetyQuality::IncompleteControlPlane,
                vec![TxFlowSafetyIssue::MissingControlPlane],
            )
        } else if stale_control_plane {
            (
                TxFlowSafetyQuality::Stale,
                vec![TxFlowSafetyIssue::StaleControlPlane],
            )
        } else if degraded_control_plane {
            (
                TxFlowSafetyQuality::Degraded,
                vec![TxFlowSafetyIssue::DegradedControlPlane],
            )
        } else {
            (TxFlowSafetyQuality::Stable, Vec::new())
        };

        TxFlowSafetySnapshot {
            quality,
            issues,
            current_state_version: effective_tip,
            replay_recovery_pending: false,
        }
    }
}

#[must_use]
pub fn direct_leader_window_slots(routing_next_leaders: usize) -> usize {
    routing_next_leaders
        .saturating_mul(LEADER_WINDOW_MULTIPLIER)
        .clamp(MIN_LEADER_WINDOW_SLOTS, MAX_LEADER_WINDOW_SLOTS)
}

#[must_use]
pub fn spawn_direct_leader_schedule_task(
    rpc_url: String,
    routing_next_leaders: usize,
    adapter: Arc<PluginHostTxProviderAdapter>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let rpc = Arc::new(RpcClient::new(rpc_url));
        let window_slots = direct_leader_window_slots(routing_next_leaders);
        let mut state = DirectLeaderScheduleState::new(rpc, adapter, window_slots);
        state.run().await;
    })
}

struct DirectLeaderScheduleState {
    rpc: Arc<RpcClient>,
    adapter: Arc<PluginHostTxProviderAdapter>,
    window_slots: usize,
    epoch_schedule: Option<EpochSchedule>,
    cached_epochs: BTreeMap<u64, Vec<LeaderScheduleEntry>>,
    last_emitted_slot: Option<u64>,
    next_rpc_retry_at: Instant,
}

impl DirectLeaderScheduleState {
    fn new(
        rpc: Arc<RpcClient>,
        adapter: Arc<PluginHostTxProviderAdapter>,
        window_slots: usize,
    ) -> Self {
        Self {
            rpc,
            adapter,
            window_slots,
            epoch_schedule: None,
            cached_epochs: BTreeMap::new(),
            last_emitted_slot: None,
            next_rpc_retry_at: Instant::now(),
        }
    }

    async fn run(&mut self) {
        let mut tick = tokio::time::interval(DIRECT_SCHEDULE_CURSOR_TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tick.tick().await;

            let observed_slot = self.observed_slot();
            let Some(observed_slot) = observed_slot else {
                continue;
            };

            if self.epoch_schedule.is_none() && self.rpc_retry_ready() {
                match self.rpc.get_epoch_schedule().await {
                    Ok(epoch_schedule) => {
                        self.epoch_schedule = Some(epoch_schedule);
                    }
                    Err(error) => {
                        log::warn!(
                            "SOF-TX direct leader-schedule bootstrap failed to fetch epoch schedule: {}",
                            error
                        );
                        self.defer_rpc_retry();
                        continue;
                    }
                }
            }

            let Some(epoch_schedule) = self.epoch_schedule.clone() else {
                continue;
            };

            let current_epoch = epoch_schedule.get_epoch(observed_slot);
            if self.rpc_retry_ready()
                && self
                    .ensure_epoch_cache(&epoch_schedule, current_epoch, observed_slot)
                    .await
                    .is_err()
            {
                self.defer_rpc_retry();
                continue;
            }

            if self.last_emitted_slot == Some(observed_slot) {
                continue;
            }

            let snapshot_leaders =
                build_window_snapshot(&self.cached_epochs, observed_slot, self.window_slots);
            if snapshot_leaders.is_empty() {
                continue;
            }

            self.adapter
                .on_leader_schedule(LeaderScheduleEvent {
                    source: ControlPlaneSource::Direct,
                    slot: Some(observed_slot),
                    epoch: Some(current_epoch),
                    added_leaders: Vec::new(),
                    removed_slots: Vec::new(),
                    updated_leaders: Vec::new(),
                    snapshot_leaders,
                    provider_source: None,
                })
                .await;
            self.last_emitted_slot = Some(observed_slot);
        }
    }

    fn observed_slot(&self) -> Option<u64> {
        let snapshot = self.adapter.control_plane_snapshot();
        snapshot
            .latest_recent_blockhash_slot
            .or(snapshot.leader_schedule_slot)
            .or(snapshot.cluster_topology_slot)
    }

    fn rpc_retry_ready(&self) -> bool {
        Instant::now() >= self.next_rpc_retry_at
    }

    fn defer_rpc_retry(&mut self) {
        self.next_rpc_retry_at = Instant::now()
            .checked_add(DIRECT_SCHEDULE_RPC_RETRY)
            .unwrap_or_else(Instant::now);
    }

    async fn ensure_epoch_cache(
        &mut self,
        epoch_schedule: &EpochSchedule,
        current_epoch: u64,
        observed_slot: u64,
    ) -> Result<(), ()> {
        if !self.cached_epochs.contains_key(&current_epoch) {
            let entries =
                fetch_epoch_schedule_window(self.rpc.as_ref(), epoch_schedule, current_epoch)
                    .await
                    .map_err(|error| {
                        log::warn!(
                            "SOF-TX direct leader-schedule refresh failed for epoch {}: {}",
                            current_epoch,
                            error
                        );
                    })?;
            log::info!(
                "SOF-TX direct loaded leader-schedule cache for epoch {} ({} slots) at observed slot {}",
                current_epoch,
                entries.len(),
                observed_slot
            );
            let _ = self.cached_epochs.insert(current_epoch, entries);
            self.next_rpc_retry_at = Instant::now();
        }

        let current_epoch_end = epoch_schedule.get_last_slot_in_epoch(current_epoch);
        let should_prefetch_next_epoch =
            current_epoch_end.saturating_sub(observed_slot) <= self.window_slots as u64;
        let next_epoch = current_epoch.saturating_add(1);
        if should_prefetch_next_epoch && !self.cached_epochs.contains_key(&next_epoch) {
            match fetch_epoch_schedule_window(self.rpc.as_ref(), epoch_schedule, next_epoch).await {
                Ok(entries) => {
                    log::info!(
                        "SOF-TX direct loaded leader-schedule cache for epoch {} ({} slots) at observed slot {}",
                        next_epoch,
                        entries.len(),
                        observed_slot
                    );
                    let _ = self.cached_epochs.insert(next_epoch, entries);
                    self.next_rpc_retry_at = Instant::now();
                }
                Err(error) => {
                    log::debug!(
                        "SOF-TX direct next-epoch leader-schedule refresh deferred for epoch {}: {}",
                        next_epoch,
                        error
                    );
                }
            }
        }

        self.cached_epochs
            .retain(|epoch, _| *epoch >= current_epoch.saturating_sub(1));
        Ok(())
    }
}

async fn fetch_epoch_schedule_window(
    rpc: &RpcClient,
    epoch_schedule: &EpochSchedule,
    epoch: u64,
) -> Result<Vec<LeaderScheduleEntry>, String> {
    let first_slot = epoch_schedule.get_first_slot_in_epoch(epoch);
    let schedule = rpc
        .get_leader_schedule(Some(first_slot))
        .await
        .map_err(|error| error.to_string())?;
    let Some(schedule) = schedule else {
        return Ok(Vec::new());
    };

    let mut entries = Vec::new();
    for (identity, relative_slots) in schedule {
        let pubkey = Pubkey::from_str(&identity)
            .map_err(|error| format!("invalid leader identity '{identity}': {error}"))?;
        let leader = pubkey.into();
        for relative_slot in relative_slots {
            let slot = first_slot.saturating_add(relative_slot as u64);
            entries.push(LeaderScheduleEntry { slot, leader });
        }
    }
    entries.sort_unstable_by_key(|entry| entry.slot);
    Ok(entries)
}

fn build_window_snapshot(
    cached_epochs: &BTreeMap<u64, Vec<LeaderScheduleEntry>>,
    observed_slot: u64,
    window_slots: usize,
) -> Vec<LeaderScheduleEntry> {
    let end_slot = observed_slot.saturating_add(window_slots as u64);
    let mut snapshot = Vec::with_capacity(window_slots);

    for entries in cached_epochs.values() {
        let start_index = entries.partition_point(|entry| entry.slot < observed_slot);
        let Some(window) = entries.get(start_index..) else {
            continue;
        };
        for entry in window {
            if entry.slot >= end_slot {
                break;
            }
            snapshot.push(*entry);
            if snapshot.len() >= window_slots {
                return snapshot;
            }
        }
    }

    snapshot
}

const fn is_slot_lag_stale(observed_slot: Option<u64>, tip_slot: u64, max_lag: u64) -> bool {
    match observed_slot {
        Some(observed_slot) => tip_slot.saturating_sub(observed_slot) > max_lag,
        None => false,
    }
}

fn control_plane_slot_spread(
    snapshot: sof_tx::adapters::TxProviderControlPlaneSnapshot,
    tip_slot: u64,
) -> u64 {
    let mut min_slot = tip_slot;
    let mut max_slot = tip_slot;

    for slot in [
        snapshot.latest_recent_blockhash_slot,
        snapshot.cluster_topology_slot,
        snapshot.leader_schedule_slot,
    ]
    .into_iter()
    .flatten()
    {
        min_slot = min_slot.min(slot);
        max_slot = max_slot.max(slot);
    }

    max_slot.saturating_sub(min_slot)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use sof::framework::{ClusterNodeInfo, ClusterTopologyEvent};
    use solana_sdk::pubkey::Pubkey;

    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn direct_window_slots_scale_with_route_depth() {
        assert_eq!(direct_leader_window_slots(0), MIN_LEADER_WINDOW_SLOTS);
        assert_eq!(direct_leader_window_slots(2), MIN_LEADER_WINDOW_SLOTS);
        assert_eq!(direct_leader_window_slots(8), MAX_LEADER_WINDOW_SLOTS);
    }

    #[test]
    fn window_snapshot_starts_at_observed_slot() {
        let entries = vec![
            LeaderScheduleEntry {
                slot: 100,
                leader: Pubkey::new_unique().into(),
            },
            LeaderScheduleEntry {
                slot: 101,
                leader: Pubkey::new_unique().into(),
            },
            LeaderScheduleEntry {
                slot: 102,
                leader: Pubkey::new_unique().into(),
            },
        ];
        let snapshot = build_window_snapshot(&BTreeMap::from([(1, entries)]), 101, 2);

        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.first().map(|entry| entry.slot), Some(101));
        assert_eq!(snapshot.get(1).map(|entry| entry.slot), Some(102));
    }

    #[tokio::test]
    async fn safety_source_requires_recent_blockhash_topology_and_schedule() {
        let adapter = Arc::new(PluginHostTxProviderAdapter::topology_only(
            Default::default(),
        ));
        let safety = DirectLeaderScheduleSafetySource::new(Arc::clone(&adapter));
        let snapshot = safety.toxic_flow_snapshot();

        assert_eq!(
            snapshot.quality,
            TxFlowSafetyQuality::IncompleteControlPlane
        );

        let leader = Pubkey::new_unique().into();
        adapter
            .on_cluster_topology(ClusterTopologyEvent {
                source: ControlPlaneSource::Direct,
                slot: Some(200),
                epoch: Some(0),
                active_entrypoint: None,
                total_nodes: 1,
                added_nodes: Vec::new(),
                removed_pubkeys: Vec::new(),
                updated_nodes: Vec::new(),
                snapshot_nodes: vec![ClusterNodeInfo {
                    pubkey: leader,
                    wallclock: 0,
                    shred_version: 0,
                    gossip: None,
                    tpu: Some(addr(9001)),
                    tpu_quic: None,
                    tpu_forwards: None,
                    tpu_forwards_quic: None,
                    tpu_vote: None,
                    tvu: None,
                    rpc: None,
                }],
                provider_source: None,
            })
            .await;
        adapter
            .on_recent_blockhash(sof::framework::ObservedRecentBlockhashEvent {
                slot: 200,
                recent_blockhash: [9_u8; 32],
                dataset_tx_count: 1,
                provider_source: None,
            })
            .await;
        adapter
            .on_leader_schedule(LeaderScheduleEvent {
                source: ControlPlaneSource::Direct,
                slot: Some(200),
                epoch: Some(0),
                added_leaders: Vec::new(),
                removed_slots: Vec::new(),
                updated_leaders: Vec::new(),
                snapshot_leaders: vec![LeaderScheduleEntry { slot: 200, leader }],
                provider_source: None,
            })
            .await;

        let stable_snapshot = safety.toxic_flow_snapshot();
        assert_eq!(stable_snapshot.quality, TxFlowSafetyQuality::Stable);
        assert_eq!(stable_snapshot.current_state_version, Some(200));
    }
}
