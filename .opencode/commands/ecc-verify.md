---
description: Run Argos format, lint, tests, and graph refresh
---
Use the ecc-verify skill to verify $ARGUMENTS. Run cargo fmt --all --check, then cargo clippy --workspace --all-targets -- -D warnings, then cargo test --workspace. If code changed, run graphify update . and report any artifact changes.
