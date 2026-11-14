DEFINE FIELD IF NOT EXISTS effective_limits_json ON actor_search_control TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS pressure_json ON actor_search_control TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS pressure_paused ON actor_search_control TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS pressure_reason ON actor_search_control TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS pressure_observed_at ON actor_search_control TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS backlog_paused ON actor_search_control TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS tail_since ON actor_search_control TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS retired_through ON actor_search_control TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS index_pause_reason ON brain_actor_seed TYPE string DEFAULT '';
DEFINE TABLE IF NOT EXISTS actor_search_rebuild SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON actor_search_rebuild TYPE string;
DEFINE FIELD IF NOT EXISTS original_job_id ON actor_search_rebuild TYPE string;
DEFINE FIELD IF NOT EXISTS request_id ON actor_search_rebuild TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON actor_search_rebuild TYPE string;
DEFINE FIELD IF NOT EXISTS expected_generation ON actor_search_rebuild TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS replacement_generation ON actor_search_rebuild TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS base_checkpoint ON actor_search_rebuild TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS visible_checkpoint ON actor_search_rebuild TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS scan_after ON actor_search_rebuild TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS scanned ON actor_search_rebuild TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS scan_complete ON actor_search_rebuild TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS scan_streak ON actor_search_rebuild TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS barrier ON actor_search_rebuild TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS pending_json ON actor_search_rebuild TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS reason_code ON actor_search_rebuild TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS repair_attempts ON actor_search_rebuild TYPE int DEFAULT 0;
-- Add defaults to an existing singleton during explicit schema bootstrap.
-- UPDATE (rather than UPSERT) keeps schema application from creating work.
UPDATE actor_search_control:catalog SET
    effective_limits_json=effective_limits_json ?? '',
    pressure_json=pressure_json ?? '',pressure_paused=pressure_paused ?? false,
    pressure_reason=pressure_reason ?? '',pressure_observed_at=pressure_observed_at ?? '',
    backlog_paused=backlog_paused ?? false,tail_since=tail_since ?? '',
    retired_through=retired_through ?? 0 RETURN NONE;
UPDATE brain_actor_seed:catalog SET index_pause_reason=index_pause_reason ?? '' RETURN NONE;
