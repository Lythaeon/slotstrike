# Runtime Architecture

## Design Summary

The runtime uses hexagonal boundaries with vertical slices:

1. `ports/`: external contracts.
2. `adapters/`: transport and persistence implementations.
3. `domain/`: settings, events, rules, value objects.
4. `slices/`: business flows (`config_sync`, strategy execution).
5. `app/`: bootstrap and composition root.

## Ingress Topology

Ingress failover chain:

1. FPGA DMA path (primary).
2. Kernel-bypass feed path (secondary).
3. Standard TCP websocket feed path (tertiary).

Bootstrap selects the first healthy path and logs selection explicitly.
Ingress feed transport is separate from tx submission transport (`direct` RPC vs `jito`).

`runtime.kernel_tcp_bypass_engine` supports `af_xdp`, `dpdk`, `openonload`, and `af_xdp_or_dpdk_external`.
For `openonload`, launch the process under Onload (or equivalent preload configuration) so socket acceleration is active.
For AF_XDP/DPDK bridge mode, provide `runtime.kernel_bypass_socket_path` and stream newline-delimited JSON frames over a unix socket.

## Event Lifecycle

1. Ingress adapter emits `RawLogEvent`.
2. Event carries `IngressMetadata`:
   - source path
   - receive timestamp
   - optional hardware timestamp
   - normalized nanosecond timestamp
3. Core engine (`SniperEngine`) classifies candidate pool-creation events.
4. Candidate events dispatch into slice handlers (`cpmm`, `openbook`).

## Deterministic Filtering

FPGA ingress applies deterministic byte-level prefiltering before payload decode:

1. OpenBook candidate requires Raydium V4 program marker and `initialize2`.
2. CPMM candidate requires standard AMM marker and excludes swap/fee/burn markers.

This reduces decode and scheduling overhead for non-candidate traffic.

## Latency Telemetry

Per-hop telemetry tracks:

1. `ingress_to_engine_ns`
2. `engine_classification_ns`
3. `strategy_dispatch_ns`

Reporter emits P50/P99/max and logs SLO alerts when P99 or max exceeds 1 ms (default).
Set `[telemetry].enabled = false` to fully disable telemetry sampling/report output.

## Replay Harness

Synthetic replay mode benchmarks burst ingestion of FPGA-style payloads and kernel-bypass log events:

```bash
cargo run --release -- --config slotstrike.toml --replay-benchmark
```

Tune with:

1. `runtime.replay_event_count`
2. `runtime.replay_burst_size`

## Boundary Rules

1. `slices` can depend on `domain` and `ports`.
2. `adapters` implement `ports` but do not import `app`.
3. `domain` is dependency-safe and contains no adapter logic.
