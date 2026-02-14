# Contributing

## Scope

This repository contains a latency-sensitive Solana sniper runtime.  
Changes should prioritize:

- correctness
- deterministic behavior
- stability under load
- measurable performance

## Development Setup

1. Install Rust stable:
```bash
rustup toolchain install stable
```
2. Install cargo-make:
```bash
cargo install cargo-make
```
3. Clone and enter the repo:
```bash
git clone <repo-url>
cd DeGeneRate
```
4. Create local runtime config:
```bash
cp sniper.example.toml sniper.toml
```

Optional (for fuzzing):

```bash
CARGO_NET_OFFLINE=false cargo install cargo-fuzz
rustup toolchain install nightly
```

## Local Quality Gates

Run these before opening a PR:

```bash
cargo make format-check
cargo make test
cargo make clippy
cargo make deny
cargo make audit
```

Optional:

- `cargo make fuzz-all` (fuzz targets)
- `cargo make test-wasm` (only if `wasm` feature/tests exist)
- `cargo make architecture-check` (only if `scripts/check_architecture.sh` exists)

## Coding Rules

- Keep domain rules explicit and type-safe (prefer newtypes for constrained values).
- Avoid floating-point math for money/percent math in execution paths.
- Prefer fixed-point / integer-safe transformations.
- Minimize allocations and lock contention on hot paths.
- Keep adapters isolated from domain logic.

## Testing Guidance

- Add unit tests for new pure logic.
- Add integration/e2e tests for behavior across boundaries.
- For parser/ingress logic, add or update fuzz targets when input surface expands.
- If changing latency-critical code, include benchmark/replay evidence where possible.

## Pull Request Process

1. Create a branch for your work.
2. Keep changes focused and atomic.
3. Update docs/config examples when behavior changes.
4. Ensure local quality gates pass.
5. Open a PR to `main` with:
   - summary of changes
   - risk notes / regressions considered
   - validation performed (tests, fuzz, replay, etc.)

## CI Expectations

PRs run automated checks (`PR Checks` workflow), including tests, lint, formatting, and dependency/security checks.

Release workflow runs on `v*` tags and can also be triggered manually.

## Release Notes and Versioning

- Use tags in the format `vX.Y.Z` (or prerelease like `vX.Y.Z-rc1`).
- Tag version must match `Cargo.toml` package version (prerelease tags compare on base version).
- If `CHANGELOG.md` exists, include a matching section for the release version.

## Security and Secrets

- Never commit private keys, RPC secrets, or tokens.
- Keep local keypairs and sensitive config out of version control.
- Use repository secrets for CI publishing (`CARGO_REGISTRY_TOKEN`).
