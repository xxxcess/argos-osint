sed -i '' 's/("retry_wait_ms", "INTEGER"),/("retry_wait_ms", "INTEGER"),\n            ("stage_coverage_json", "TEXT NOT NULL DEFAULT '\''{}'\''"),/g' crates/argos-osint-core/src/tasks.rs
