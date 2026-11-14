//! Bounded CSV header preflight and origin-relative authenticated upload tickets.

use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;

use serde::Deserialize;
use serde::Serialize;

use crate::readiness::DatasetKind;

pub const DATASET_UPLOADS_PATH: &str = "/api/v1/workstation/datasets/uploads";
pub const MAX_CSV_HEADER_BYTES: u64 = 64 * 1024;
pub const ACTOR_CSV_COLUMNS: [&str; 5] = [
    "actor_id",
    "actor_name",
    "actor_type",
    "aliases",
    "name_normalized",
];
pub const TRAINING_CSV_COLUMNS: [&str; 18] = [
    "sample_id",
    "title",
    "description",
    "domain_labels",
    "primary_domain",
    "secondary_domain",
    "subcategories",
    "source_kind",
    "difficulty",
    "dataset_version",
    "annotation_notes",
    "split",
    "scenario_group",
    "language",
    "synthetic",
    "labels_complete",
    "expected_admission",
    "rejection_reason",
];

/// Maximum exact bytes accepted by the corresponding backend dataset owner.
pub const fn dataset_upload_limit(kind: DatasetKind) -> u64 {
    match kind {
        DatasetKind::ActorCatalog => 256 * 1024 * 1024,
        DatasetKind::ClassifierTraining => 128 * 1024 * 1024,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatasetUploadContractError;

impl fmt::Display for DatasetUploadContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid dataset upload contract")
    }
}

impl std::error::Error for DatasetUploadContractError {}

/// Request an upload destination without transferring dataset bytes or a local path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DatasetUploadRequest {
    pub kind: DatasetKind,
    pub sha256: String,
    pub size_bytes: u64,
}

impl DatasetUploadRequest {
    pub fn new(
        kind: DatasetKind,
        sha256: String,
        size_bytes: u64,
    ) -> Result<Self, DatasetUploadContractError> {
        let request = Self {
            kind,
            sha256: sha256.to_ascii_lowercase(),
            size_bytes,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), DatasetUploadContractError> {
        if self.size_bytes == 0
            || self.size_bytes > dataset_upload_limit(self.kind)
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(DatasetUploadContractError);
        }
        Ok(())
    }
}

/// Relative destination; possession never replaces current registered authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DatasetUploadTicket {
    pub upload_path: String,
    pub expires_in_seconds: u64,
}

impl DatasetUploadTicket {
    pub fn new(
        upload_path: String,
        expires_in_seconds: u64,
    ) -> Result<Self, DatasetUploadContractError> {
        let ticket = Self {
            upload_path,
            expires_in_seconds,
        };
        ticket.validate()?;
        Ok(ticket)
    }

    pub fn validate(&self) -> Result<(), DatasetUploadContractError> {
        let id = self
            .upload_path
            .strip_prefix(DATASET_UPLOADS_PATH)
            .and_then(|suffix| suffix.strip_prefix('/'))
            .ok_or(DatasetUploadContractError)?;
        if !(1..=64).contains(&id.len())
            || !id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
            || !(1..=3600).contains(&self.expires_in_seconds)
        {
            return Err(DatasetUploadContractError);
        }
        Ok(())
    }
}

/// Header diagnostics contain only fixed messages or known required column names.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CsvHeaderError {
    Read,
    TooLarge,
    Empty,
    Malformed,
    MissingColumns(Vec<&'static str>),
    DuplicateColumns,
    UnexpectedColumns,
}

impl fmt::Display for CsvHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => formatter.write_str("Could not read the CSV header."),
            Self::TooLarge => formatter.write_str("CSV header exceeds 64 KiB."),
            Self::Empty => formatter.write_str("CSV header is empty."),
            Self::Malformed => formatter.write_str("CSV header is malformed or is not UTF-8."),
            Self::MissingColumns(columns) => {
                write!(
                    formatter,
                    "CSV header is missing required columns: {}.",
                    columns.join(", ")
                )
            }
            Self::DuplicateColumns => formatter.write_str("CSV header contains duplicate columns."),
            Self::UnexpectedColumns => {
                formatter.write_str("Training CSV header contains unexpected columns.")
            }
        }
    }
}

impl std::error::Error for CsvHeaderError {}

/// Validate only the first CSV record. Full row validation remains server-owned.
///
/// Reads at most 64 KiB + 1; callers must rewind before hashing or uploading.
/// One BOM, quoted fields and CRLF are handled by the same CSV parser as storage.
pub fn validate_csv_header(
    reader: &mut impl Read,
    kind: DatasetKind,
) -> Result<(), CsvHeaderError> {
    let mut csv = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(reader.take(MAX_CSV_HEADER_BYTES + 1));
    let header = csv
        .headers()
        .map_err(|error| {
            if error.is_io_error() {
                CsvHeaderError::Read
            } else {
                CsvHeaderError::Malformed
            }
        })?
        .clone();
    if csv.position().byte() > MAX_CSV_HEADER_BYTES {
        return Err(CsvHeaderError::TooLarge);
    }
    if header.is_empty() {
        return Err(CsvHeaderError::Empty);
    }
    let required: &[&'static str] = match kind {
        DatasetKind::ActorCatalog => &ACTOR_CSV_COLUMNS,
        DatasetKind::ClassifierTraining => &TRAINING_CSV_COLUMNS,
    };
    let mut columns = BTreeSet::new();
    for column in &header {
        if !columns.insert(column) {
            return Err(CsvHeaderError::DuplicateColumns);
        }
    }
    let missing: Vec<_> = required
        .iter()
        .copied()
        .filter(|column| !columns.contains(column))
        .collect();
    if !missing.is_empty() {
        return Err(CsvHeaderError::MissingColumns(missing));
    }
    if kind == DatasetKind::ClassifierTraining
        && columns.iter().any(|column| !required.contains(column))
    {
        return Err(CsvHeaderError::UnexpectedColumns);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_bom_crlf_and_header_only_are_valid_headers() {
        for (kind, columns) in [
            (DatasetKind::ActorCatalog, ACTOR_CSV_COLUMNS.as_slice()),
            (
                DatasetKind::ClassifierTraining,
                TRAINING_CSV_COLUMNS.as_slice(),
            ),
        ] {
            let quoted = columns
                .iter()
                .map(|column| format!("\"{column}\""))
                .collect::<Vec<_>>()
                .join(",");
            let raw = format!("\u{feff}{quoted}\r\n");
            validate_csv_header(&mut raw.as_bytes(), kind).unwrap();
        }
        let extra = format!("{},extra\n", ACTOR_CSV_COLUMNS.join(","));
        validate_csv_header(&mut extra.as_bytes(), DatasetKind::ActorCatalog).unwrap();
    }

    #[test]
    fn missing_duplicate_wrong_purpose_and_extra_training_columns_are_rejected() {
        for kind in [DatasetKind::ActorCatalog, DatasetKind::ClassifierTraining] {
            assert_eq!(
                validate_csv_header(&mut "".as_bytes(), kind),
                Err(CsvHeaderError::Empty)
            );
            assert!(matches!(
                validate_csv_header(&mut "private-unknown-name\n".as_bytes(), kind),
                Err(CsvHeaderError::MissingColumns(_))
            ));
            let duplicate = format!("{},actor_id\n", ACTOR_CSV_COLUMNS.join(","));
            assert_eq!(
                validate_csv_header(&mut duplicate.as_bytes(), kind),
                Err(CsvHeaderError::DuplicateColumns)
            );
        }
        let actors = ACTOR_CSV_COLUMNS.join(",");
        assert!(
            validate_csv_header(&mut actors.as_bytes(), DatasetKind::ClassifierTraining).is_err()
        );
        let extra = format!("{},private-unknown-name\n", TRAINING_CSV_COLUMNS.join(","));
        let error = validate_csv_header(&mut extra.as_bytes(), DatasetKind::ClassifierTraining)
            .unwrap_err();
        assert_eq!(error, CsvHeaderError::UnexpectedColumns);
        assert!(!error.to_string().contains("private-unknown-name"));
    }

    #[test]
    fn header_allocation_is_bounded_even_without_a_terminator() {
        let raw = vec![b'x'; MAX_CSV_HEADER_BYTES as usize + 1024];
        let mut input = std::io::Cursor::new(raw);
        assert_eq!(
            validate_csv_header(&mut input, DatasetKind::ActorCatalog),
            Err(CsvHeaderError::TooLarge)
        );
        assert_eq!(input.position(), MAX_CSV_HEADER_BYTES + 1);
        let mut invalid: &[u8] = &[0xff, b'\n'];
        assert_eq!(
            validate_csv_header(&mut invalid, DatasetKind::ActorCatalog),
            Err(CsvHeaderError::Malformed)
        );
    }

    #[test]
    fn upload_bounds_and_exact_origin_relative_ticket_paths_are_enforced() {
        assert!(DatasetUploadRequest::new(
            DatasetKind::ActorCatalog,
            "a".repeat(64),
            256 * 1024 * 1024
        )
        .is_ok());
        assert!(DatasetUploadRequest::new(
            DatasetKind::ClassifierTraining,
            "a".repeat(64),
            128 * 1024 * 1024 + 1
        )
        .is_err());
        assert!(DatasetUploadRequest::new(DatasetKind::ActorCatalog, "a".repeat(64), 0).is_err());
        assert!(
            DatasetUploadRequest::new(DatasetKind::ActorCatalog, "not-a-hash".into(), 1).is_err()
        );
        assert!(DatasetUploadTicket::new(
            format!("{DATASET_UPLOADS_PATH}/{}", "a".repeat(64)),
            300
        )
        .is_ok());
        for path in [
            "https://other.invalid/api/v1/workstation/datasets/uploads/id",
            "//other.invalid/api/v1/workstation/datasets/uploads/id",
            "/api/v1/workstation/datasets/uploads/../id",
            "/api/v1/workstation/datasets/uploads/%2e%2e",
            "/api/v1/workstation/datasets/uploads/id?token=private",
            "/api/v1/workstation/datasets/uploads/id#fragment",
            "/api/v1/workstation/datasets/uploads/",
            "/api/v1/workstation/datasets/uploads/id/extra",
        ] {
            assert!(DatasetUploadTicket::new(path.into(), 300).is_err());
        }
        assert!(DatasetUploadTicket::new(format!("{DATASET_UPLOADS_PATH}/id"), 0).is_err());
    }
}
