#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

CONFIG_PATH="${CONFIG_PATH:-sniper.toml}"
DEVNET_RPC_URL="${DEVNET_RPC_URL:-https://api.devnet.solana.com}"
DEVNET_WSS_URL="${DEVNET_WSS_URL:-wss://api.devnet.solana.com}"
KEYPAIR_PATH="${KEYPAIR_PATH:-keypair.devnet.json}"
SMOKE_TIMEOUT_SECS="${SMOKE_TIMEOUT_SECS:-180}"
PRIORITY_FEES="${PRIORITY_FEES:-1000}"
SNIPE_HEIGHT_SOL="${SNIPE_HEIGHT_SOL:-0.01}"
TIP_BUDGET_SOL="${TIP_BUDGET_SOL:-0.001}"
SLIPPAGE_PCT="${SLIPPAGE_PCT:-1}"
DEVNET_TARGET_MINT="${DEVNET_TARGET_MINT:-}"
MANUAL_FUND_WAIT_SECS="${MANUAL_FUND_WAIT_SECS:-900}"

require_cmd() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Missing required command: ${command_name}" >&2
    exit 1
  fi
}

first_mint_from_config() {
  local file_path="$1"
  if [[ ! -f "${file_path}" ]]; then
    return 0
  fi

  awk '
    /^\[\[rules\]\]/ {
      if (in_rule && kind == "mint" && address != "") {
        print address
        exit
      }
      in_rule=1
      kind=""
      address=""
      next
    }
    in_rule && $1 == "kind" {
      value=$3
      gsub(/"/, "", value)
      kind=tolower(value)
      next
    }
    in_rule && $1 == "address" {
      value=$3
      gsub(/"/, "", value)
      address=value
      next
    }
    END {
      if (in_rule && kind == "mint" && address != "") {
        print address
      }
    }
  ' "${file_path}"
}

ensure_keypair() {
  if [[ -f "${KEYPAIR_PATH}" ]]; then
    echo "Using existing keypair: ${KEYPAIR_PATH}"
    return 0
  fi

  echo "Creating new devnet keypair: ${KEYPAIR_PATH}"
  solana-keygen new \
    --no-bip39-passphrase \
    --silent \
    --force \
    --outfile "${KEYPAIR_PATH}" >/dev/null
}

current_balance_sol() {
  local pubkey="$1"
  local raw_balance
  raw_balance="$(solana balance "${pubkey}" --url "${DEVNET_RPC_URL}" 2>/dev/null || true)"
  awk '{print $1}' <<<"${raw_balance}"
}

fund_with_one_sol() {
  local pubkey="$1"
  local balance
  balance="$(current_balance_sol "${pubkey}")"
  echo "Current balance for ${pubkey}: ${balance:-0} SOL"

  if awk "BEGIN { exit !(${balance:-0} >= 1.0) }"; then
    echo "Balance already >= 1 SOL, skipping airdrop."
    return 0
  fi

  local attempt
  for attempt in $(seq 1 5); do
    echo "Airdrop attempt ${attempt}/5: requesting 1 SOL on devnet..."
    if ! solana airdrop 1 "${pubkey}" --url "${DEVNET_RPC_URL}"; then
      echo "Airdrop attempt ${attempt} failed (likely faucet rate limit)." >&2
    fi

    local wait_index
    for wait_index in $(seq 1 5); do
      balance="$(current_balance_sol "${pubkey}")"
      if awk "BEGIN { exit !(${balance:-0} >= 1.0) }"; then
        echo "Balance after funding: ${balance} SOL"
        return 0
      fi
      sleep 2
    done
  done

  echo "Automatic airdrop attempts exhausted."
  echo "Manual funding required."
  echo "1. Open: https://faucet.solana.com"
  echo "2. Select network: devnet"
  echo "3. Wallet address: ${pubkey}"
  echo "4. Request at least 1 SOL"

  local elapsed=0
  while (( elapsed < MANUAL_FUND_WAIT_SECS )); do
    balance="$(current_balance_sol "${pubkey}")"
    if awk "BEGIN { exit !(${balance:-0} >= 1.0) }"; then
      echo "Manual funding detected. Balance: ${balance} SOL"
      return 0
    fi

    sleep 5
    elapsed=$((elapsed + 5))
  done

  echo "Wallet balance is still below 1 SOL after manual wait timeout (${MANUAL_FUND_WAIT_SECS}s)." >&2
  echo "Latest balance: ${balance:-0} SOL" >&2
  exit 1
}

resolve_target_mint() {
  local mint_from_config
  mint_from_config="$(first_mint_from_config "${CONFIG_PATH}")"

  if [[ -n "${DEVNET_TARGET_MINT}" ]]; then
    echo "${DEVNET_TARGET_MINT}"
    return 0
  fi

  if [[ -n "${mint_from_config}" ]]; then
    echo "${mint_from_config}"
    return 0
  fi

  echo "No mint rule found in ${CONFIG_PATH} and DEVNET_TARGET_MINT was not set." >&2
  echo "Set DEVNET_TARGET_MINT=<mint_address> and rerun." >&2
  exit 1
}

prepare_smoke_config() {
  local target_mint="$1"
  TMP_SMOKE_CONFIG_PATH="$(mktemp /tmp/sniper.devnet.smoke.XXXXXX.toml)"
  export TMP_SMOKE_CONFIG_PATH

  cat > "${TMP_SMOKE_CONFIG_PATH}" <<EOF
[runtime]
keypair_path = "${KEYPAIR_PATH}"
rpc_url = "${DEVNET_RPC_URL}"
wss_url = "${DEVNET_WSS_URL}"
priority_fees = ${PRIORITY_FEES}
tx_submission_mode = "direct"
jito_url = "${DEVNET_RPC_URL}"
kernel_tcp_bypass = false
kernel_tcp_bypass_engine = "af_xdp_or_dpdk_external"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[telemetry]
sample_capacity = 4096
slo_ns = 1000000
report_period_secs = 15

[[rules]]
kind = "mint"
address = "${target_mint}"
snipe_height_sol = "${SNIPE_HEIGHT_SOL}"
tip_budget_sol = "${TIP_BUDGET_SOL}"
slippage_pct = "${SLIPPAGE_PCT}"
EOF
}

cleanup() {
  if [[ -n "${TMP_SMOKE_CONFIG_PATH:-}" && -f "${TMP_SMOKE_CONFIG_PATH}" ]]; then
    rm -f "${TMP_SMOKE_CONFIG_PATH}"
  fi
}

run_smoke_snipe() {
  local keypair_pubkey="$1"

  echo "Starting devnet smoke snipe with keypair ${KEYPAIR_PATH} (${keypair_pubkey})"
  echo "Config: ${TMP_SMOKE_CONFIG_PATH}"
  echo "RPC: ${DEVNET_RPC_URL}"
  echo "WSS: ${DEVNET_WSS_URL}"

  set +e
  timeout "${SMOKE_TIMEOUT_SECS}" \
    cargo run --release -- --config "${TMP_SMOKE_CONFIG_PATH}"
  local run_status=$?
  set -e

  if [[ "${run_status}" -eq 124 ]]; then
    echo "Smoke timeout reached (${SMOKE_TIMEOUT_SECS}s). Sniper started and ran, but no matching snipe event was observed in-window."
    return 0
  fi

  if [[ "${run_status}" -ne 0 ]]; then
    echo "Sniper run failed with exit code: ${run_status}" >&2
    return "${run_status}"
  fi

  echo "Sniper run completed successfully."
}

main() {
  require_cmd "cargo"
  require_cmd "solana"
  require_cmd "solana-keygen"
  require_cmd "timeout"

  solana config set --url "${DEVNET_RPC_URL}" >/dev/null

  ensure_keypair
  local pubkey
  pubkey="$(solana-keygen pubkey "${KEYPAIR_PATH}")"

  fund_with_one_sol "${pubkey}"
  local target_mint
  target_mint="$(resolve_target_mint)"
  echo "Using target mint: ${target_mint}"

  prepare_smoke_config "${target_mint}"
  trap cleanup EXIT

  run_smoke_snipe "${pubkey}"
}

main "$@"
