//! One error classification for server and embedded SurrealDB.
//!
//! SurrealDB 3.3.4 reports a write conflict differently per engine: the
//! server returns kind `Query` with `QueryError::TransactionConflict`, while
//! the embedded RocksDB engine returns kind `Internal` with no details and only
//! the message "Transaction conflict: ... This transaction can be retried".
//! [`normalize`] turns the embedded form into the structured one so callers
//! and retry helpers match a single typed variant in both modes.

use surrealdb::types::QueryError;
use surrealdb::Error;

/// Message prefix the 3.3.4 engines use for a retryable write conflict.
const CONFLICT_MARKER: &str = "Transaction conflict";
const RETRY_MARKER: &str = "can be retried";

/// True when `error` is a retryable write conflict, in either form.
pub fn is_conflict(error: &Error) -> bool {
    error.query_details() == Some(&QueryError::TransactionConflict)
        || is_unstructured_conflict(error)
}

fn is_unstructured_conflict(error: &Error) -> bool {
    error.is_internal()
        && error.message().contains(CONFLICT_MARKER)
        && error.message().contains(RETRY_MARKER)
}

/// Return `error` with a write conflict in structured form
/// (`Query` / `QueryError::TransactionConflict`). Every other error is
/// returned unchanged.
pub fn normalize(error: Error) -> Error {
    if is_unstructured_conflict(&error) {
        Error::query(error.message().to_string(), QueryError::TransactionConflict)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBEDDED: &str = "Transaction conflict: Resource busy. This transaction can be retried";

    #[test]
    fn embedded_conflict_becomes_structured() {
        let raw = Error::internal(EMBEDDED.to_string());
        assert!(is_conflict(&raw));
        let mapped = normalize(raw);
        assert!(mapped.is_query());
        assert_eq!(
            mapped.query_details(),
            Some(&QueryError::TransactionConflict)
        );
        assert_eq!(mapped.message(), EMBEDDED);
    }

    #[test]
    fn structured_conflict_is_unchanged() {
        let raw = Error::query("conflict".to_string(), QueryError::TransactionConflict);
        assert!(is_conflict(&raw));
        assert_eq!(
            normalize(raw).query_details(),
            Some(&QueryError::TransactionConflict)
        );
    }

    #[test]
    fn other_errors_are_not_conflicts() {
        for raw in [
            Error::internal("Database index `x` already contains 'k'".to_string()),
            Error::internal("Transaction conflict without the retry hint".to_string()),
            Error::query("timed out".to_string(), QueryError::Cancelled),
        ] {
            assert!(!is_conflict(&raw), "{}", raw.message());
            let kind = raw.kind_str();
            assert_eq!(normalize(raw).kind_str(), kind);
        }
    }
}
