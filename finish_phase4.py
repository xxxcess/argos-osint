import re
import os

GRAPH_FILE = "crates/argos-osint-core/src/recon/graph.rs"
with open(GRAPH_FILE, "r") as f:
    content = f.read()

# Add OptionalExtension to imports
content = re.sub(r'use std::collections::{HashMap, HashSet};', 'use std::collections::{HashMap, HashSet};\nuse rusqlite::OptionalExtension;', content)

# Fix type annotation
content = content.replace('|r| r.get(0)', '|r| r.get::<_, String>(0)')

with open(GRAPH_FILE, "w") as f:
    f.write(content)

PUB_FILE = "crates/argos-osint-core/src/store/publication.rs"
with open(PUB_FILE, "r") as f:
    pub_content = f.read()

pub_patch = """                        let memory_id = new_id();
                        self.conn.execute(
                            "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                            params![memory_id, sentence, now, source_json],
                        )?;
                        self.conn.execute("INSERT INTO memory_metadata (memory_id, memory_kind) VALUES (?1, 'atomic_claim') ON CONFLICT(memory_id) DO UPDATE SET memory_kind=excluded.memory_kind", params![memory_id])?;"""
pub_content = pub_content.replace("""                        let memory_id = new_id();
                        self.conn.execute(
                            "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES (?1,?2,'investigation',0,?3,?4)",
                            params![memory_id, sentence, now, source_json],
                        )?;""", pub_patch)

brief_patch = """            if let Some(brief_id) = &receipt.brief_memory_id {
                self.conn.execute("INSERT INTO memory_metadata (memory_id, memory_kind) VALUES (?1, 'cycle_brief') ON CONFLICT(memory_id) DO UPDATE SET memory_kind=excluded.memory_kind", params![brief_id])?;
                for (_, claim_mem_id) in &receipt.claim_memory_ids {
                    self.conn.execute("INSERT INTO memory_brief_memberships (brief_id, member_id, member_type) VALUES (?1, ?2, 'atomic_claim') ON CONFLICT DO NOTHING", params![brief_id, claim_mem_id])?;
                }
            }"""
pub_content = pub_content.replace("""            if let Some(brief_id) = &receipt.brief_memory_id {
                if !affected.contains(brief_id) {
                    affected.push(brief_id.clone());
                }
            }""", brief_patch + """            if let Some(brief_id) = &receipt.brief_memory_id {
                if !affected.contains(brief_id) {
                    affected.push(brief_id.clone());
                }
            }""")

with open(PUB_FILE, "w") as f:
    f.write(pub_content)

print("Patched.")
