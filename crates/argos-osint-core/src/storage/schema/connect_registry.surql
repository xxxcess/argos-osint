DEFINE TABLE IF NOT EXISTS connect_registry_guard SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON connect_registry_guard TYPE int DEFAULT 0;

DEFINE TABLE IF NOT EXISTS connect_control SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS configuration_epoch ON connect_control TYPE string;
DEFINE FIELD IF NOT EXISTS admission_open ON connect_control TYPE bool DEFAULT true;

DEFINE TABLE IF NOT EXISTS connect_workstation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workstation_id ON connect_workstation TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON connect_workstation TYPE string;
DEFINE FIELD IF NOT EXISTS installation_hash ON connect_workstation TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_ip ON connect_workstation TYPE string;
DEFINE FIELD IF NOT EXISTS state ON connect_workstation TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON connect_workstation TYPE int;
DEFINE FIELD IF NOT EXISTS last_seen_at ON connect_workstation TYPE datetime;
DEFINE FIELD IF NOT EXISTS observed_peer_ip ON connect_workstation TYPE option<string>;
DEFINE FIELD IF NOT EXISTS observed_peer_at ON connect_workstation TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS connect_installation_unique ON connect_workstation FIELDS scope_id, installation_hash UNIQUE;

DEFINE TABLE IF NOT EXISTS connect_ip_claim SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workstation_ip ON connect_ip_claim TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON connect_ip_claim TYPE string;
DEFINE INDEX IF NOT EXISTS connect_ip_unique ON connect_ip_claim FIELDS workstation_ip UNIQUE;

DEFINE TABLE IF NOT EXISTS connect_workspace SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workspace_id ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_name ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS marker_hash ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS protected_default ON connect_workspace TYPE bool;
DEFINE FIELD IF NOT EXISTS state ON connect_workspace TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON connect_workspace TYPE int;
DEFINE INDEX IF NOT EXISTS connect_workspace_marker_unique ON connect_workspace FIELDS workstation_id, marker_hash UNIQUE;

DEFINE TABLE IF NOT EXISTS connect_name_claim SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS workstation_id ON connect_name_claim TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_name ON connect_name_claim TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_id ON connect_name_claim TYPE string;
DEFINE INDEX IF NOT EXISTS connect_name_unique ON connect_name_claim FIELDS workstation_id, workspace_name UNIQUE;

DEFINE TABLE IF NOT EXISTS connect_registration SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS registration_id ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_id ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_ip ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_name ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS connect_id ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS state ON connect_registration TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON connect_registration TYPE int;
DEFINE FIELD IF NOT EXISTS created_at ON connect_registration TYPE datetime;
DEFINE INDEX IF NOT EXISTS connect_registration_workspace ON connect_registration FIELDS workspace_id, created_at;

DEFINE TABLE IF NOT EXISTS connect_binding SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS connect_id ON connect_binding TYPE string;
DEFINE FIELD IF NOT EXISTS registration_id ON connect_binding TYPE string;
DEFINE INDEX IF NOT EXISTS connect_binding_unique ON connect_binding FIELDS connect_id UNIQUE;

DEFINE TABLE IF NOT EXISTS connect_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS request_hash ON connect_request TYPE string;
DEFINE FIELD IF NOT EXISTS registration_id ON connect_request TYPE string;

DEFINE TABLE IF NOT EXISTS connect_session SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope_id ON connect_session TYPE string;
DEFINE FIELD IF NOT EXISTS credential_id ON connect_session TYPE string;
DEFINE FIELD IF NOT EXISTS access_epoch ON connect_session TYPE string;
DEFINE FIELD IF NOT EXISTS configuration_epoch ON connect_session TYPE string;
DEFINE FIELD IF NOT EXISTS registration_id ON connect_session TYPE string;
DEFINE FIELD IF NOT EXISTS registration_revision ON connect_session TYPE int;
DEFINE FIELD IF NOT EXISTS context_revision ON connect_session TYPE int;
DEFINE FIELD IF NOT EXISTS scope_revision ON connect_session TYPE int;
DEFINE FIELD IF NOT EXISTS created_at ON connect_session TYPE datetime;
DEFINE FIELD IF NOT EXISTS expires_at ON connect_session TYPE datetime;
DEFINE FIELD IF NOT EXISTS revoked ON connect_session TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS connect_session_registration ON connect_session FIELDS registration_id, revoked;
DEFINE INDEX IF NOT EXISTS connect_session_expiry ON connect_session FIELDS expires_at;

DEFINE TABLE IF NOT EXISTS connect_rate_window SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS started_at ON connect_rate_window TYPE datetime;
DEFINE FIELD IF NOT EXISTS attempts ON connect_rate_window TYPE int;

-- Version-two migration deliberately overwrites old required folder/IP fields.
-- Legacy records retain their values; new registrations have no folder identity.
DEFINE FIELD OVERWRITE contract_version ON connect_control TYPE option<int>;
DEFINE FIELD OVERWRITE workstation_ip ON connect_workstation TYPE option<string>;
DEFINE FIELD OVERWRITE workspace_id ON connect_registration TYPE option<string>;
DEFINE FIELD OVERWRITE workspace_name ON connect_registration TYPE option<string>;
DEFINE FIELD OVERWRITE workstation_ip ON connect_registration TYPE option<string>;
DEFINE FIELD OVERWRITE contract_version ON connect_registration TYPE option<int>
    ASSERT $this.state != 'active' OR $value = 2;
DEFINE FIELD OVERWRITE connect_id ON connect_registration TYPE string
    ASSERT $this.state != 'active' OR $value = $this.workstation_id;
DEFINE INDEX IF NOT EXISTS connect_registration_workstation ON connect_registration FIELDS workstation_id, contract_version;
