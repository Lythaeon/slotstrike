#!/usr/bin/env bash
set -euo pipefail

if ! cargo fuzz --version >/dev/null 2>&1; then
  cat >&2 <<'EOF'
cargo-fuzz is not installed.
Install it once with:
  CARGO_NET_OFFLINE=false cargo install cargo-fuzz
EOF
  exit 1
fi

if ! rustup toolchain list | grep -q '^nightly'; then
  cat >&2 <<'EOF'
nightly toolchain is not installed.
Install it once with:
  rustup toolchain install nightly
EOF
  exit 1
fi
