---
description: Run Argos format, lint, tests, and graph refresh
---
Use the ecc-verify skill to verify $ARGUMENTS. Keep ARGOS_EMBED unset. Run cargo fmt --all --check, then cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings, then cargo test --workspace --locked --no-default-features. For TUI changes also use argos-tui-verify and docs/tui-verification.md; capture and inspect actual fixture PNGs, report interactions and metric tests separately. If code changed, run graphify update . and report artifact changes.
