DEFINE TABLE IF NOT EXISTS configuration_reset SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope_id ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS reset_id ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS plan_sha256 ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS deployment_id ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS old_configuration_epoch ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS old_access_epoch ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS configuration_epoch ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS access_epoch ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS inventory_json ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS revoked_json ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS local_receipts_json ON configuration_reset TYPE string;
DEFINE FIELD IF NOT EXISTS configuration_revision ON configuration_reset TYPE int;
DEFINE FIELD IF NOT EXISTS operator_credential_id ON configuration_reset TYPE string;
DEFINE INDEX IF NOT EXISTS configuration_reset_identity ON configuration_reset FIELDS scope_id,reset_id UNIQUE;

DEFINE TABLE IF NOT EXISTS configuration_reset_scope SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope_id ON configuration_reset_scope TYPE string;
DEFINE FIELD IF NOT EXISTS active_reset ON configuration_reset_scope TYPE string;
DEFINE FIELD IF NOT EXISTS access_epoch ON configuration_reset_scope TYPE string;
DEFINE FIELD IF NOT EXISTS first_use_blocked ON configuration_reset_scope TYPE bool;
DEFINE FIELD IF NOT EXISTS revoked_json ON configuration_reset_scope TYPE string;
DEFINE INDEX IF NOT EXISTS configuration_reset_scope_identity ON configuration_reset_scope FIELDS scope_id UNIQUE;

-- A token cannot regain access by being relabeled with another scope or epoch.
-- These are one-way digests from actually loaded policy, never bearer bytes.
DEFINE TABLE IF NOT EXISTS configuration_reset_revoked SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS fingerprint ON configuration_reset_revoked TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON configuration_reset_revoked TYPE string;
DEFINE FIELD IF NOT EXISTS reset_id ON configuration_reset_revoked TYPE string;
DEFINE INDEX IF NOT EXISTS configuration_reset_revoked_fingerprint ON configuration_reset_revoked FIELDS fingerprint UNIQUE;
