DEFINE TABLE IF NOT EXISTS setup_dataset SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dataset_id ON setup_dataset TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON setup_dataset TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON setup_dataset TYPE string;
DEFINE FIELD IF NOT EXISTS sha256 ON setup_dataset TYPE string;
DEFINE FIELD IF NOT EXISTS size_bytes ON setup_dataset TYPE int;
DEFINE FIELD IF NOT EXISTS validated ON setup_dataset TYPE bool;
DEFINE FIELD IF NOT EXISTS pin_revision ON setup_dataset TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON setup_dataset TYPE datetime;
DEFINE FIELD IF NOT EXISTS expires_at ON setup_dataset TYPE datetime;
DEFINE INDEX IF NOT EXISTS setup_dataset_scope ON setup_dataset FIELDS scope_id, created_at;
DEFINE INDEX IF NOT EXISTS setup_dataset_expiry ON setup_dataset FIELDS expires_at;

DEFINE TABLE IF NOT EXISTS setup_dataset_pin SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dataset_id ON setup_dataset_pin TYPE string;
DEFINE FIELD IF NOT EXISTS job_id ON setup_dataset_pin TYPE string;
DEFINE INDEX IF NOT EXISTS setup_dataset_pin_unique ON setup_dataset_pin FIELDS dataset_id, job_id UNIQUE;
DEFINE INDEX IF NOT EXISTS setup_dataset_pin_job ON setup_dataset_pin FIELDS job_id;

DEFINE TABLE IF NOT EXISTS setup_upload SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dataset_id ON setup_upload TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON setup_upload TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON setup_upload TYPE string;
DEFINE FIELD IF NOT EXISTS state ON setup_upload TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON setup_upload TYPE datetime;
DEFINE INDEX IF NOT EXISTS setup_upload_expiry ON setup_upload FIELDS state, expires_at;

DEFINE TABLE IF NOT EXISTS setup_operation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS operation ON setup_operation TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON setup_operation TYPE string;
DEFINE FIELD IF NOT EXISTS payload_hash ON setup_operation TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON setup_operation TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON setup_operation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON setup_operation TYPE datetime;

DEFINE TABLE IF NOT EXISTS setup_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS payload_hash ON setup_request TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON setup_request TYPE string;

DEFINE TABLE IF NOT EXISTS setup_resource SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON setup_resource TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS job_id ON setup_resource TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS payload_hash ON setup_resource TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS scope_id ON setup_resource TYPE string DEFAULT '';

DEFINE TABLE IF NOT EXISTS setup_dispatch SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS enabled ON setup_dispatch TYPE bool DEFAULT false;
