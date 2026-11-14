DEFINE TABLE IF NOT EXISTS classifier_activation_operation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope_id ON classifier_activation_operation TYPE string;
DEFINE FIELD IF NOT EXISTS request_json ON classifier_activation_operation TYPE string;
DEFINE FIELD IF NOT EXISTS request_hash ON classifier_activation_operation TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON classifier_activation_operation TYPE string;
DEFINE FIELD IF NOT EXISTS state ON classifier_activation_operation TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON classifier_activation_operation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS started_at ON classifier_activation_operation TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS updated_at ON classifier_activation_operation TYPE datetime;

DEFINE TABLE IF NOT EXISTS classifier_activation_reconciliation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON classifier_activation_reconciliation TYPE string;
DEFINE FIELD IF NOT EXISTS request_hash ON classifier_activation_reconciliation TYPE string;
DEFINE FIELD IF NOT EXISTS origin_json ON classifier_activation_reconciliation TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON classifier_activation_reconciliation TYPE string;
