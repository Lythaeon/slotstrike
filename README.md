# DeGeneRate Slotstrike

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
- Ingress path failover (log/event intake):
  - FPGA DMA (primary)
  - Kernel bypass feed path (secondary)
  - Standard TCP websocket feed path (fallback)

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
tx_submission_mode = "jito"
jito_url = "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/transactions?bundleOnly=true"
kernel_tcp_bypass = true
kernel_tcp_bypass_engine = "af_xdp_or_dpdk_external"
kernel_bypass_socket_path = "/tmp/slotstrike-kernel-bypass.sock"

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
- `wss_url`: websocket URL.
- `priority_fees`: microlamports.
- `tx_submission_mode`: `jito` or `direct`.
- `jito_url`: required when `tx_submission_mode = "jito"`.
- `kernel_tcp_bypass`: enable kernel bypass path.
- `kernel_tcp_bypass_engine`: `af_xdp`, `dpdk`, `openonload`, `af_xdp_or_dpdk_external`.
- `kernel_bypass_socket_path`: unix socket for external AF_XDP/DPDK bypass feed bridge.
- `fpga_enabled`: force FPGA ingress if available.
- `fpga_verbose`: verbose FPGA diagnostics.
- `fpga_vendor`: vendor label.
- `replay_benchmark`: run synthetic replay instead of live strategy.
- `replay_event_count`: replay event count.
- `replay_burst_size`: replay burst size.

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

External kernel-bypass bridge frame format (newline-delimited JSON on unix socket):

```json
{"signature":"...","logs":["..."],"has_error":false,"hardware_timestamp_ns":1700000000,"received_timestamp_ns":1700000100}
```

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
- `--fpga`
- `--fpga-verbose`

If you set `kernel_tcp_bypass_engine = "openonload"`, run under Onload (or equivalent preload setup) to activate acceleration.

Note: ingress feed transport and tx submission transport are separate concerns.  
Ingress can use FPGA/kernel-bypass/websocket, while tx submission uses `direct` RPC or `jito` based on `tx_submission_mode`.

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
- Clippy (deny warnings): `cargo make clippy`
- Cargo deny: `cargo make deny`
- Fuzz all targets: `cargo make fuzz-all`

Fuzz prerequisites (one-time):

- `CARGO_NET_OFFLINE=false cargo install cargo-fuzz`
- `rustup toolchain install nightly`

## Benchmarking

Synthetic replay benchmark:

```bash
cargo make replay-benchmark
```

Tune with:

- `runtime.replay_event_count`
- `runtime.replay_burst_size`

## Documentation

- Runtime architecture: `docs/architecture/runtime.md`
- On-call playbook: `docs/runbooks/oncall.md`
- Contribution guide: `CONTRIBUTING.md`
- FPGA NIC deployment/PTP/rollback: `docs/operations/fpga_nic_deployment.md`

## Disclaimer

Use at your own risk. You are responsible for all trading decisions, infrastructure security, and financial outcomes.
