# FPGA Direct Ingress Contract

## Purpose

This document defines the production contract for `fpga_ingress_mode = "direct_device"` in slotstrike.
It is intended for operators integrating a vendor FPGA/NIC driver directly into runtime ingress.

## Runtime Configuration

Set these `[runtime]` fields in `slotstrike.toml`:

```toml
fpga_enabled = true
fpga_vendor = "xilinx" # generic | exanic | xilinx | amd | solarflare | napatech | mock_dma
fpga_ingress_mode = "direct_device" # auto | mock_dma | direct_device | external_socket
fpga_direct_device_path = "/dev/slotstrike-fpga0"
fpga_dma_socket_path = "/tmp/slotstrike-fpga-dma.sock" # only used by external_socket mode
```

`auto` resolves to:

1. `mock_dma` backend when `fpga_vendor = "mock_dma"`.
2. `direct_device` backend for supported hardware vendors.

## Readiness Rules (Fail Fast)

At startup, slotstrike rejects FPGA mode when prerequisites fail:

1. Unsupported vendor/mode combination.
2. Missing `fpga_direct_device_path`.
3. Path exists but is not a char device, FIFO, or readable file.
4. Path exists but cannot be opened.

The process exits before wallet/RPC/rulebook initialization if ingress readiness fails.

## Direct Device Wire Format

The direct device reader consumes one frame per line.

Preferred line format (JSON):

```json
{"payload_base64":"c2lnbmF0dXJlPWFiYzEyMwo...","hardware_timestamp_ns":1730000000123456789}
```

Allowed alternatives:

1. JSON with `payload` (plain UTF-8 string) instead of `payload_base64`.
2. Payload-only base64 line (no JSON envelope).

Each payload is decoded by slotstrike's deterministic DMA parser and then passed through pool prefiltering.

## Payload Contract

Decoded payload must contain newline-separated fields:

```text
signature=<tx_signature>
has_error=<0|1|true|false>
log=<solana log line 1>
log=<solana log line 2>
...
```

If payload parsing fails, slotstrike drops that frame and continues reading.

## Operational Notes

1. Keep the device producer line-buffered to avoid burst latency from partial frames.
2. Use hardware timestamps where available and include `hardware_timestamp_ns` in the JSON envelope.
3. Prefer character device or FIFO paths over regular files in production.
4. For rollback, switch `fpga_enabled = false` and restart service.

