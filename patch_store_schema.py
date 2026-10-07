import re

with open("crates/argos-osint-core/src/store.rs", "r") as f:
    text = f.read()

text = text.replace("pub const SCHEMA_VERSION: i64 = 21;", "pub const SCHEMA_VERSION: i64 = 22;")

migration_code = """
            if version < 22 {
                self.conn.execute_batch(
                    "UPDATE intel_report_jobs
                     SET tool_calls_done = -1
                     WHERE state IN ('completed', 'failed', 'partial', 'cancelled')
                       AND tool_calls_done = 0;"
                )?;
                self.conn.pragma_update(None, "user_version", 22)?;
            }
            // Additive, idempotent: revision-aware graph summary cache and
"""

text = text.replace("            // Additive, idempotent: revision-aware graph summary cache and", migration_code.strip() + "\n")

with open("crates/argos-osint-core/src/store.rs", "w") as f:
    f.write(text)

