# Slotstrike

High-performance Solana slotstrike runtime focused on Raydium pool creation events.

## Scope

- Supported pools:
  - Raydium OpenBook
  - Raydium CPMM
- Rule targeting:
  - Mint address
  - Deployer address
- Transaction submission modes:
  - `jito`
  - `direct`
- SOF ingress/runtime modes:
  - websocket provider-stream
  - Yellowstone gRPC provider-stream
  - private shred ingest with trusted/untrusted posture
- Runtime stance:
  - SOF-only ingress and structured transaction parsing
  - legacy raw-log ingress removed on March 29, 2026

## Configuration Model

This project is TOML-only. `.env` is not used by runtime configuration.

1. Copy `slotstrike.example.toml` to `slotstrike.toml`.
2. Edit the runtime and rule sections.
3. Start the binary with `--config`.

Minimal example:

```toml
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://api.mainnet-beta.solana.com"
wss_url = "wss://api.mainnet-beta.solana.com"
priority_fees = 1000000
dry_run = false
tx_submission_mode = "jito"
jito_url = "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/transactions?bundleOnly=true"

[sof]
enabled = true
source = "websocket"
websocket_url = "wss://api.mainnet-beta.solana.com"
private_shred_socket_path = "/tmp/slotstrike-sof-private-shreds.sock"
private_shred_source_addr = "127.0.0.1:19001"
trusted_private_shreds = false
commitment = "processed"
inline_transaction_dispatch = true
ingest_queue_mode = "lockfree"
ingest_queue_capacity = 16384

[sof_tx]
enabled = true
mode = "custom"
strategy = "ordered_fallback"
routes = ["jito", "rpc"]
jito_transport = "json_rpc"
bundle_only = true

[telemetry]
enabled = true
sample_capacity = 4096
slo_ns = 1000000
report_period_secs = 15

[[rules]]
kind = "mint"
address = "So11111111111111111111111111111111111111112"
snipe_height_sol = "0.01"
tip_budget_sol = "0.001"
slippage_pct = "1"

[[rules]]
kind = "mint"
address = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
snipe_height_sol = "0.01"
tip_budget_sol = "0.001"
slippage_pct = "1"
```

## Config Reference

`[runtime]`:

- `keypair_path`: path to Solana keypair JSON.
- `rpc_url`: HTTP RPC URL.
- `wss_url`: compatibility alias for SOF websocket mode. Prefer `sof.websocket_url`.
- `priority_fees`: microlamports.
- `dry_run`: build and sign swaps without submitting them.
- `tx_submission_mode`: `jito` or `direct`.
- `jito_url`: required when `tx_submission_mode = "jito"`.
- `replay_benchmark`: run synthetic replay instead of live strategy.
- `replay_event_count`: replay event count.
- `replay_burst_size`: replay burst size.

Legacy note:

- Legacy ingress runtime keys such as old `kernel_tcp_bypass` and `fpga_*` settings are no longer accepted. The runtime config is SOF-only and those fields now fail TOML parsing instead of being silently ignored.

`[sof]`:

- `enabled`: enables SOF-backed runtime startup.
- `source`: `websocket`, `grpc`, or `private_shred`.
- `websocket_url`: provider websocket endpoint for SOF websocket mode.
- `grpc_url`: Yellowstone gRPC endpoint for SOF gRPC mode.
- `grpc_x_token`: optional Yellowstone auth token.
- `private_shred_socket_path`: unix socket path Slotstrike binds for private shred propagation.
- `private_shred_source_addr`: synthetic source address attached to packets arriving from the private shred socket.
- `gossip_entrypoints`: optional SOF gossip bootstrap entrypoints used for direct-route TPU topology.
- `gossip_validators`: optional validator allowlist for gossip control-plane tracking.
- `gossip_runtime_mode`: `full`, `bootstrap_only`, or `control_plane_only`. Slotstrike defaults to `control_plane_only` so gossip maintains topology/leader state without gossip shred ingest.
- `trusted_private_shreds`: when `true`, SOF uses trusted raw-shred provider mode.
- `commitment`: `processed`, `confirmed`, or `finalized`.
- `inline_transaction_dispatch`: enables SOF inline transaction dispatch where possible.
- `startup_step_logs`, `worker_threads`, `dataset_workers`, `packet_workers`: runtime tuning knobs.
- `ingest_queue_mode`: `bounded`, `unbounded`, or `lockfree`.
- `ingest_queue_capacity`: queue capacity for bounded/lockfree ingest.

`[sof_tx]`:

- `enabled`: enables `sof-tx` submission from the strategy path.
- `mode`: `rpc`, `jito`, `direct`, `hybrid`, or `custom`.
- `strategy`: `ordered_fallback` or `all_at_once`.
- `routes`: explicit route order for `mode = "custom"`.
- `reliability`: `low_latency`, `balanced`, or `high_reliability`.
- `jito_transport`: `json_rpc` or `grpc`.
- `jito_endpoint`: optional Jito block-engine endpoint override.
- `bundle_only`: enables Jito revert protection.
- `routing_next_leaders`, `routing_backup_validators`, `routing_max_parallel_sends`: direct-route tuning.
- `guard_require_stable_control_plane`, `guard_reject_on_replay_recovery_pending`, `guard_max_state_version_drift`, `guard_max_opportunity_age_ms`, `guard_suppression_ttl_ms`: toxic-flow guard policy.
- Direct-route TPU topology is sourced from SOF gossip bootstrap via `sof.gossip_entrypoints`.
- Direct-route leader schedule is refreshed infrequently from `runtime.rpc_url` and injected into `sof-tx`.
- `sof.gossip_runtime_mode = "control_plane_only"` keeps that control plane active without enabling gossip shred ingest.

Guard rail:

- `sof_tx` direct routing can be used with `websocket`, `grpc`, or `private_shred`.
- Direct routing requires `runtime.rpc_url` plus at least one `sof.gossip_entrypoints` value.
- Gossip supplies TPU topology; Slotstrike uses SOF recent-blockhash state to advance the direct leader window and RPC leader-schedule snapshots to keep routing accurate.
- `private_shred` remains the lowest-latency option for direct routing, but the direct control plane can now stay gossip topology-only instead of ingesting gossip shreds.

`[telemetry]`:

- `enabled`: if `false`, telemetry sampling/reporter logs are disabled.
- `sample_capacity`: per-hop sample buffer size.
- `slo_ns`: SLO threshold in nanoseconds.
- `report_period_secs`: telemetry report interval.

`[[rules]]`:

- `kind`: `mint` or `deployer`.
- `address`: target pubkey.
- `snipe_height_sol`: SOL amount string.
- `tip_budget_sol`: SOL amount string.
- `slippage_pct`: percent string.

Note: monetary/percentage rule values are strings and parsed via fixed-point/integer-safe logic to avoid float drift.

Multiple mint addresses:

Use one `[[rules]]` block per mint address (the `rules` section is an array of tables).

```toml
[[rules]]
kind = "mint"
address = "So11111111111111111111111111111111111111112"
snipe_height_sol = "0.01"
tip_budget_sol = "0.001"
slippage_pct = "1"

[[rules]]
kind = "mint"
address = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
snipe_height_sol = "0.02"
tip_budget_sol = "0.001"
slippage_pct = "1.5"
```

What telemetry is:

Telemetry is internal latency instrumentation for core pipeline hops:

- ingress to engine (`ingress_to_engine_ns`)
- engine classification (`engine_classification_ns`)
- strategy dispatch (`strategy_dispatch_ns`)

How telemetry is shown:

- periodic `info` logs: `Latency telemetry > hop=... count=... p50=... p99=... max=...`
- `warn` logs on SLO breaches: `Latency SLO alert > ...`
- under systemd, view via `journalctl -u <service-name>`

Disable telemetry completely:

```toml
[telemetry]
enabled = false
```

When disabled, no telemetry samples or telemetry report lines are emitted.

## Run

Direct:

```bash
cargo run --release -- --config slotstrike.toml
```

Via cargo-make:

```bash
cargo make run-slotstrike -- --config slotstrike.toml
```

Useful runtime flags:

- `--config <path>`
- `--replay-benchmark`

Note: ingress feed transport and tx submission transport are separate concerns.  
Slotstrike now always uses SOF for ingress/runtime selection and can route submits through `sof-tx`. The legacy `tx_submission_mode` remains only as a compatibility fallback when `sof_tx.enabled = false`.

## Linux systemd

Service registration is built into the binary.

Install + enable + start:

```bash
sudo cargo run --release -- --install-service --config /home/slotstrike/slotstrike/slotstrike.toml
```

Uninstall:

```bash
sudo cargo run --release -- --uninstall-service
```

Optional install flags:

- `--service-name <name>` (default `slotstrike`)
- `--service-user <user>` (default from `SUDO_USER`/`USER`)
- `--service-group <group>` (default user primary group)
- `--systemd-dir <path>` (default `/etc/systemd/system`)
- `--no-enable` (write unit only, do not enable/start)

Cargo-make wrappers:

```bash
sudo cargo make service-install -- --config /home/slotstrike/slotstrike/slotstrike.toml
sudo cargo make service-uninstall
```

## Devnet Smoke Test

Runs a usability smoke pass using direct submission (no Jito dependency).

```bash
cargo make devnet-smoke
```

Auto bootstrap (creates keypair, attempts funding to 1 SOL, then runs smoke):

```bash
cargo make devnet-auto-snipe
```

Optional environment overrides:

- `CONFIG_PATH`
- `DEVNET_TARGET_MINT`
- `KEYPAIR_PATH`
- `DEVNET_RPC_URL`
- `DEVNET_WSS_URL`
- `SMOKE_TIMEOUT_SECS`

If automatic airdrop is rate-limited, the script prompts manual funding via `https://faucet.solana.com`.

## Quality Gates

- Tests (nextest): `cargo make test`
- Live read-only replay probe: `SLOTSTRIKE_LIVE_SIGNATURE=<sig> SLOTSTRIKE_LIVE_KIND=cpmm|openbook SLOTSTRIKE_LIVE_MINT=<mint> cargo test --test live_raydium_dry_run -- --ignored --nocapture`
- Clippy: `cargo make clippy`
- Fuzz all targets: `cargo make fuzz-all`

Fuzz prerequisites (one-time):

- `CARGO_NET_OFFLINE=false cargo install cargo-fuzz`
- `rustup toolchain install nightly`

## Benchmarking

Synthetic replay benchmark:

```bash
cargo make replay-benchmark
```

Symbolized release profile:

```bash
CARGO_PROFILE_RELEASE_STRIP=none perf record -o perf-slotstrike.data -- target/release/slotstrike --config slotstrike.example.toml --replay-benchmark
perf report --stdio --no-children -i perf-slotstrike.data
```

Tune with:

- `runtime.replay_event_count`
- `runtime.replay_burst_size`

Current measured replay costs on this host with `events=50000` and `repeats=20`:

- `sof_structured_creation_scan`: `25,580,254 ev/s`, `p50 20ns`, `p99 30ns`
- `sof_structured_swap_scan`: `25,368,739 ev/s`, `p50 20ns`, `p99 40ns`

Out-of-the-box SOF improvement versus the old cold legacy baseline on this same host:

- Creation path:
  `25,580,254 ev/s` with SOF vs `1,959,388 ev/s` on old FPGA DMA and `3,861,559 ev/s` on old kernel-bypass.
- Exact creation speedup:
  `13.06x` vs old FPGA DMA, `6.62x` vs old kernel-bypass.
- Swap-reject path:
  `25,368,739 ev/s` with SOF vs `2,548,307 ev/s` on old FPGA DMA and `3,808,408 ev/s` on old kernel-bypass.
- Exact swap-reject speedup:
  `9.96x` vs old FPGA DMA, `6.66x` vs old kernel-bypass.

Interpretation:

- Slotstrike no longer ships the legacy raw-log ingress path. The active runtime is always structured and SOF-native.
- Historical cold legacy baseline on this same host, captured earlier on March 29, 2026 before the legacy finder-reuse pass:
  `fpga_dma_creation_scan = 1,959,388 ev/s`, `kernel_bypass_creation_scan = 3,861,559 ev/s`,
  `fpga_dma_swap_scan = 2,548,307 ev/s`, `kernel_bypass_swap_scan = 3,808,408 ev/s`.
- `private_shred` should be preferred when you trust the feed and want the lowest end-to-end latency plus SOF-TX direct routing support.
- `grpc` is the next-best operational choice when you want the same structured Slotstrike processing path without running private shred infrastructure.
- `websocket` is still valid through SOF, but its transport/decode layer is typically the least attractive of the SOF options for ultra-low-latency use.
- The SOF candidate plugin is instruction-aware for Raydium pool creation and only forwards structured CPMM/OpenBook create transactions into the sniper path.
- The strategy handlers no longer refetch `JsonParsed` transactions or scan RPC log strings. They parse instruction data and accounts directly from the SOF transaction, resolving ALT lookups only when needed.
- ALT lookup tables are now cached in-process and only refetched when a transaction references indexes beyond the cached table length, which removes repeated `getMultipleAccounts` churn from the live v0 path.
- The latest symbolized profile is benchmark-noise dominated. Slotstrike replay symbols fell below the `0.5%` report threshold; the top samples were VDSO clock reads plus allocator/page-fault overhead from the synthetic harness.

## Documentation

- Runtime architecture: `docs/architecture/runtime.md`
- On-call playbook: `docs/runbooks/oncall.md`
- Contribution guide: `CONTRIBUTING.md`

## Disclaimer

Use at your own risk. You are responsible for all trading decisions, infrastructure security, and financial outcomes.
