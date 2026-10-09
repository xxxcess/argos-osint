---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---
Run `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings`, then `cargo test --workspace --locked --no-default-features`. Keep `ARGOS_EMBED` unset. Do not pass `--features lancedb`. After code changes, run `graphify update .`. Report each result and changed graph artifacts.
