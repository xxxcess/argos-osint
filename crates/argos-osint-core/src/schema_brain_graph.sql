-- Schema 37: unified Brain memory graph (typed kinds, actors, endpoints) and
-- per-stage Intel brief timing. Additive; legacy tables stay as projections.

CREATE TABLE IF NOT EXISTS brain_actors (
    actor_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    subtype TEXT NOT NULL DEFAULT 'unknown',
    display_name TEXT NOT NULL,
    identity_state TEXT NOT NULL DEFAULT 'provisional',
    -- Deterministic scope of a provisional actor (e.g. source/run); '' once resolved.
    scope_key TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 1,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS brain_actors_scope ON brain_actors(scope_key);

-- Names are search keys, never identity: aliases are nonunique.
CREATE TABLE IF NOT EXISTS brain_actor_aliases (
    actor_id TEXT NOT NULL REFERENCES brain_actors(actor_id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    normalized TEXT NOT NULL,
    alias_type TEXT NOT NULL DEFAULT 'name',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (actor_id, normalized, alias_type)
);
CREATE INDEX IF NOT EXISTS brain_actor_alias_lookup
    ON brain_actor_aliases(normalized, alias_type);

CREATE TABLE IF NOT EXISTS brain_actor_mentions (
    id TEXT PRIMARY KEY,
    actor_id TEXT NOT NULL REFERENCES brain_actors(actor_id) ON DELETE CASCADE,
    origin_kind TEXT NOT NULL,
    origin_scope TEXT NOT NULL DEFAULT '',
    origin_id TEXT NOT NULL,
    origin_revision TEXT NOT NULL DEFAULT '',
    spans_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS brain_actor_mentions_actor ON brain_actor_mentions(actor_id);
CREATE INDEX IF NOT EXISTS brain_actor_mentions_origin
    ON brain_actor_mentions(origin_kind, origin_id);

CREATE TABLE IF NOT EXISTS brain_actor_external_refs (
    scope TEXT NOT NULL,
    legacy_id TEXT NOT NULL,
    actor_id TEXT NOT NULL REFERENCES brain_actors(actor_id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (scope, legacy_id)
);
CREATE INDEX IF NOT EXISTS brain_actor_external_actor ON brain_actor_external_refs(actor_id);

-- Assessed merges; acyclicity is enforced by the writer.
CREATE TABLE IF NOT EXISTS brain_actor_redirects (
    from_actor TEXT PRIMARY KEY,
    to_actor TEXT NOT NULL,
    receipt_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS brain_memory_actor_refs (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL,
    role TEXT NOT NULL,
    binding_state TEXT NOT NULL DEFAULT 'provisional',
    provenance_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (memory_id, actor_id, role)
);
CREATE INDEX IF NOT EXISTS brain_memory_actor_refs_actor ON brain_memory_actor_refs(actor_id);

-- Typed endpoints of Link/Related memories (edge ID = memory ID).
CREATE TABLE IF NOT EXISTS brain_memory_endpoints (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    side TEXT NOT NULL CHECK(side IN ('source','target')),
    endpoint_kind TEXT NOT NULL CHECK(endpoint_kind IN ('memory','article','source')),
    ref_id TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT '',
    revision TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (memory_id, side)
);
CREATE INDEX IF NOT EXISTS brain_memory_endpoints_ref
    ON brain_memory_endpoints(endpoint_kind, ref_id);

-- Legacy row → canonical memory map (migration receipts).
CREATE TABLE IF NOT EXISTS brain_legacy_map (
    legacy_table TEXT NOT NULL,
    legacy_id TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    migrated_at TEXT NOT NULL,
    PRIMARY KEY (legacy_table, legacy_id)
);

CREATE TABLE IF NOT EXISTS intel_brief_stage_timings (
    task_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    queue_ms INTEGER NOT NULL DEFAULT 0,
    local_ms INTEGER NOT NULL DEFAULT 0,
    admission_ms INTEGER NOT NULL DEFAULT 0,
    first_output_ms INTEGER NOT NULL DEFAULT 0,
    headers_ms INTEGER NOT NULL DEFAULT 0,
    generation_ms INTEGER NOT NULL DEFAULT 0,
    fidelity_ms INTEGER NOT NULL DEFAULT 0,
    backoff_ms INTEGER NOT NULL DEFAULT 0,
    persistence_ms INTEGER NOT NULL DEFAULT 0,
    total_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    routes_json TEXT NOT NULL DEFAULT '[]',
    outcome TEXT NOT NULL DEFAULT '',
    recorded_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS intel_brief_timing_job ON intel_brief_stage_timings(job_id);

-- Schema 38: resumable local Brain backfill receipts (per source unit) and
-- per-source high-water state.
CREATE TABLE IF NOT EXISTS brain_backfill_receipts (
    source TEXT NOT NULL,
    unit_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    coverage TEXT NOT NULL DEFAULT '',
    completed_at TEXT NOT NULL,
    PRIMARY KEY (source, unit_id)
);

-- Atlas phase 6 receipt: one per article occurrence, written atomically with
-- the occurrence's actor bindings, mention Links and bounded Related records.
CREATE TABLE IF NOT EXISTS atlas_article_index (
    run_id TEXT NOT NULL,
    article_id TEXT NOT NULL,
    input_revision TEXT NOT NULL,
    index_checksum TEXT NOT NULL,
    actor_coverage TEXT NOT NULL DEFAULT '',
    indexed_at TEXT NOT NULL,
    PRIMARY KEY (run_id, article_id)
);

-- Schema 40 lifecycle guards (spec §5 "Backfill and lifecycle"): operational
-- cleanup and ad-hoc deletes cannot remove a canonical Actor that is still
-- pinned, mentioned, referenced or part of a merge lineage (retire instead),
-- and deleting any memory retires Link/Related records that pointed at it.
CREATE TRIGGER IF NOT EXISTS brain_guard_actor_delete
BEFORE DELETE ON memories
WHEN EXISTS (SELECT 1 FROM brain_actors a WHERE a.actor_id = OLD.id)
 AND (OLD.pinned = 1
   OR EXISTS (SELECT 1 FROM brain_actor_mentions m WHERE m.actor_id = OLD.id)
   OR EXISTS (SELECT 1 FROM brain_memory_actor_refs r WHERE r.actor_id = OLD.id AND r.memory_id <> OLD.id)
   OR EXISTS (SELECT 1 FROM brain_actor_redirects d WHERE d.from_actor = OLD.id OR d.to_actor = OLD.id))
BEGIN
    SELECT RAISE(ABORT, 'brain: canonical actor is still referenced; retire it instead of deleting');
END;

CREATE TRIGGER IF NOT EXISTS brain_retire_dangling_edges
AFTER DELETE ON memories
BEGIN
    UPDATE memory_metadata SET lifecycle = 'retired'
     WHERE memory_kind IN ('link', 'related')
       AND memory_id IN (SELECT memory_id FROM brain_memory_endpoints
                          WHERE endpoint_kind = 'memory' AND ref_id = OLD.id);
END;
