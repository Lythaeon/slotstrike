# On-Call Runbook

## Alert Categories

1. `Latency SLO alert` (P99/max above threshold).
2. Ingress failover warnings (`FPGA ingress unavailable`, `Kernel bypass ingress unavailable`).
3. Swap execution failures (transaction send/status errors).

## First 5 Minutes

1. Confirm active ingress path:
```bash
journalctl -u sniper -n 200 | rg "Ingress path selected|ingress unavailable"
```

2. Check latency telemetry trends:
```bash
journalctl -u sniper -n 400 | rg "Latency telemetry|Latency SLO alert"
```
If no telemetry lines appear, verify `[telemetry].enabled` in `sniper.toml`.

3. Validate RPC and websocket reachability from host.

## Ingress Path Incidents

### FPGA Path Degraded

1. Verify NIC/PTP health.
2. If unstable, set in `sniper.toml` `[runtime]`:
```bash
fpga_enabled=false
kernel_tcp_bypass=true
```
3. Restart service.

### Kernel Bypass Unavailable

1. Validate `runtime.kernel_tcp_bypass_engine`.
   Supported values: `af_xdp`, `dpdk`, `openonload`, `af_xdp_or_dpdk_external`.
2. If AF_XDP/DPDK external bridge mode is selected, verify `runtime.kernel_bypass_socket_path` exists and producer is running.
3. If unsupported or degraded, set in `sniper.toml` `[runtime]`:
```bash
kernel_tcp_bypass=false
```
4. Restart service; runtime will use standard TCP.

## Latency Spike Playbook

1. Confirm whether spike is ingress-only or dispatch-wide using hop metrics.
2. Reduce external contention:
   - isolate host CPU
   - verify NIC IRQ pinning
   - reduce noisy background workloads
3. If unresolved, force fallback to simpler path and compare telemetry.

## Replay Benchmark for Diagnostics

Use synthetic harness to compare local path characteristics:

```bash
cargo run --release -- --config sniper.toml --replay-benchmark
```

If FPGA path underperforms kernel-bypass path in replay, treat as ingress processing regression.

## Escalation

Escalate to infra/network team when:

1. Repeated FPGA initialization failures.
2. PTP drift above 1 ms for more than 5 minutes.
3. Both kernel bypass and standard stream startup fail.

## Incident Closure Checklist

1. Confirm stable ingress path for 15 minutes.
2. Confirm no active SLO alerts for 15 minutes.
3. Document root cause, mitigation, and follow-up action items.
