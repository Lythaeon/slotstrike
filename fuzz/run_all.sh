#!/usr/bin/env bash
set -euo pipefail

max_seconds="${1:-30}"
targets=(
  "fuzz_log_parser"
  "fuzz_pool_filter"
  "fuzz_sol_amount"
  "fuzz_rule_primitives"
  "fuzz_fpga_dma_payload"
)

for target in "${targets[@]}"; do
  echo "Running ${target} for ${max_seconds}s"
  cargo +nightly fuzz run "${target}" -- -max_total_time="${max_seconds}"
done
