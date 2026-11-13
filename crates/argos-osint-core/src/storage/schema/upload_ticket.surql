-- Destination metadata only: no bearer, polling secret, or filesystem path.
DEFINE TABLE IF NOT EXISTS upload_ticket SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS issuer_key ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS credential_id ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS access_epoch ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS purpose ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS context_json ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS sha256 ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS size_bytes ON upload_ticket TYPE int;
DEFINE FIELD IF NOT EXISTS state ON upload_ticket TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON upload_ticket TYPE datetime;
DEFINE FIELD IF NOT EXISTS expires_at ON upload_ticket TYPE datetime;
DEFINE FIELD IF NOT EXISTS claimed_at ON upload_ticket TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS upload_ticket_issuer_expiry ON upload_ticket FIELDS issuer_key,expires_at;
DEFINE INDEX IF NOT EXISTS upload_ticket_expiry ON upload_ticket FIELDS expires_at;

-- One stable quota guard per authenticated scope/credential/epoch/purpose.
DEFINE TABLE IF NOT EXISTS upload_ticket_issuer SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON upload_ticket_issuer TYPE int DEFAULT 0;
