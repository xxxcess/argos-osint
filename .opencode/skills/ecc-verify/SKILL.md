---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---
Run `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`. Keep `ARGOS_EMBED` unset for offline tests. After code changes, run `graphify update .`. Report each result and changed graph artifacts.
