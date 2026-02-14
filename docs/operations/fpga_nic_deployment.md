# FPGA NIC Deployment, PTP Sync, and Rollback

## Scope

This runbook covers:

1. FPGA NIC firmware deployment.
2. PTP clock synchronization verification.
3. Safe rollback to previous firmware and transport path.

## Preconditions

1. Maintenance window approved.
2. Out-of-band access to host (IPMI/ILO or console).
3. Backup of current NIC firmware image and toolchain.
4. `runtime.kernel_tcp_bypass = true` and `runtime.fpga_enabled = false` available as immediate fallback.
5. Latest known-good `slotstrike.toml` copy stored before changes.

## Deployment Steps

1. Stop slotstrike service:
```bash
systemctl stop slotstrike
```

2. Capture baseline state:
```bash
ethtool -i <nic_ifname>
phc_ctl /dev/ptp0 get
```

3. Flash FPGA NIC firmware using vendor tooling:
```bash
<vendor_tool> flash --device <nic_ifname> --image <firmware_image>
```

4. Reboot host if vendor requires reboot for bitstream activation.

5. Verify firmware activation:
```bash
ethtool -i <nic_ifname>
<vendor_tool> status --device <nic_ifname>
```

6. Enable FPGA path in `slotstrike.toml` under `[runtime]`:
```bash
fpga_enabled = true
fpga_vendor = "<vendor_name>"
fpga_verbose = false
```

7. Start service and confirm ingress:
```bash
systemctl start slotstrike
journalctl -u slotstrike -f
```

Expected logs include `Ingress path selected: FPGA DMA ring`.

## PTP Clock Sync Validation

1. Start/verify `ptp4l` and `phc2sys`:
```bash
systemctl status ptp4l
systemctl status phc2sys
```

2. Validate drift is within target:
```bash
pmc -u -b 0 'GET TIME_STATUS_NP'
phc_ctl /dev/ptp0 cmp
```

3. Reject deployment if offset exceeds 1 ms for sustained intervals.

## Rollback Procedure

1. Switch runtime path to non-FPGA immediately in `slotstrike.toml` `[runtime]`:
```bash
fpga_enabled = false
kernel_tcp_bypass = true
```

2. Restart service:
```bash
systemctl restart slotstrike
```

3. If firmware issue persists, flash previous known-good image:
```bash
<vendor_tool> flash --device <nic_ifname> --image <last_known_good_image>
```

4. Re-validate NIC and PTP health with commands from sections above.

5. Record incident timeline and attach logs from:
```bash
journalctl -u slotstrike --since "30 min ago"
```

## Post-Change Checks

1. Confirm no continuous `Latency SLO alert` warnings.
2. Confirm strategy event ingestion is uninterrupted for 15 minutes.
3. Document firmware version, host, operator, and rollback readiness in change ticket.
