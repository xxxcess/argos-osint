//! Bounded, read-only resolution and validation of synthetic classifier CSVs.
//!
//! Resolution never reads CSV contents. Parsing is synchronous file/CPU work;
//! async callers must offload it. CSV annotations cannot mint reviewed-real
//! provenance, and the customer dataset is never rewritten by this module.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::training::{self, DatasetRow, FamilyId, LabelMembership, Split, TrainingCorpus};
use super::{ClassifierError, Provenance};
use crate::atlas_taxonomy::DomainId;

/// Exact runtime seed filename; no wildcard or actor-corpus fallback.
pub const DATASET_FILE_NAME: &str = "argos_atlas_strategic_domains_v2.csv";
/// Version whose complete release invariants are mandatory.
pub const SEED_DATASET_VERSION: &str = "strategic-domains-v1-seed-1";
/// Distinct-scenario corpus with four permanent partitions and one/two-tag admission.
pub const DISTINCT_DATASET_VERSION: &str = "strategic-domains-v2-distinct-1";
/// Maximum exact input bytes, including BOM and line endings.
pub const MAX_CSV_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum raw record bytes, including quoting and embedded newlines.
pub const MAX_CSV_RECORD_BYTES: usize = 128 * 1024;
/// Maximum decoded bytes for every non-feature field.
pub const MAX_METADATA_FIELD_BYTES: usize = 16 * 1024;
const HEADER: [&str; 18] = [
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

/// Selection source, in descending precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DatasetCandidate {
    Explicit,
    Environment,
    Configured,
    RepoSingular,
    RepoPlural,
    Home,
}

/// Captured runtime inputs; pure resolution performs no environment reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DatasetRequest {
    pub explicit_file: Option<PathBuf>,
    pub environment_file: Option<PathBuf>,
    pub configured_file: Option<PathBuf>,
    pub cwd: PathBuf,
    pub executable: PathBuf,
    pub home: PathBuf,
}

impl DatasetRequest {
    /// Captures environment and runtime paths once, without creating directories.
    pub fn from_runtime(explicit_file: Option<PathBuf>) -> Result<Self, DatasetError> {
        let cwd =
            std::env::current_dir().map_err(|e| io_error(Path::new("."), "current_dir", e))?;
        let executable = std::env::current_exe().map_err(|e| io_error(&cwd, "current_exe", e))?;
        let home = crate::paths::home_dir();
        let home = if home.is_absolute() {
            home
        } else {
            cwd.join(home)
        };
        let paths = crate::data_paths::DataPaths::load_from_home(&home)
            .map_err(|error| io_error(&home.join("config.toml"), "read_config", error))?;
        Ok(Self {
            explicit_file,
            environment_file: std::env::var_os("ARGOS_CLASSIFIER_CSV").map(PathBuf::from),
            configured_file: crate::data_paths::DataPaths::resolve(&home, &paths.article_csv),
            cwd,
            executable,
            home,
        })
    }
}

/// Bounded diagnostic without CSV row contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DatasetDiagnostic {
    pub code: String,
    pub message: String,
}
/// One exact candidate inspected during resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CandidateProbe {
    pub candidate: DatasetCandidate,
    pub path: PathBuf,
    pub diagnostic: Option<DatasetDiagnostic>,
}
/// File metadata is a cache hint, never cryptographic identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileFingerprint {
    pub length: u64,
    pub modified_ns: Option<u128>,
    pub changed_ns: Option<u128>,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}
/// Canonical selected path and resolution evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DatasetResolution {
    pub candidate: DatasetCandidate,
    pub path: PathBuf,
    pub searched: Vec<CandidateProbe>,
    pub fingerprint: FileFingerprint,
}
/// Validation profile derived from the file's data version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CsvProfile {
    DeliveredSeedV1,
    DeliveredDistinctV2,
    SyntheticDevelopment,
}
/// Actual row/family counts. Split arrays are train, calibration, development, test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DatasetSummary {
    pub rows: u64,
    pub families: u64,
    pub split_rows: [u64; 4],
    pub split_families: [u64; 4],
    pub positive_rows: [[u64; 6]; 4],
    pub dataset_sha256: String,
}
/// Fully validated rows and canonical U2 hashes, ready for training/evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParsedDataset {
    pub resolution: DatasetResolution,
    pub profile: CsvProfile,
    pub dataset_version: String,
    pub corpus: TrainingCorpus,
    pub summary: DatasetSummary,
}

/// Typed parse/selection failure. Displays never include full record contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatasetError {
    NotFound {
        searched: Vec<CandidateProbe>,
    },
    InvalidSelection {
        candidate: DatasetCandidate,
        path: PathBuf,
        reason: String,
    },
    Io {
        path: PathBuf,
        operation: &'static str,
        reason: String,
    },
    TooLarge {
        path: PathBuf,
        limit: u64,
        kind: String,
    },
    Csv {
        row: u64,
        field: Option<String>,
        code: String,
    },
    Schema {
        missing: Vec<String>,
        extra: Vec<String>,
        duplicate: Vec<String>,
    },
    InvalidRow {
        row: u64,
        sample_id: Option<String>,
        field: String,
        code: String,
    },
    FamilyConflict {
        family_id: String,
        code: String,
    },
    ReleaseCounts {
        invariant: String,
        expected: u64,
        actual: u64,
    },
    ChangedDuringRead {
        path: PathBuf,
    },
}
impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { .. } => f.write_str("classifier dataset not found"),
            Self::InvalidSelection { path, reason, .. } => write!(f, "invalid dataset selection {}: {reason}", path.display()),
            Self::Io { path, operation, reason } => write!(f, "dataset {operation} failed for {}: {reason}", path.display()),
            Self::TooLarge { limit, kind, .. } => write!(f, "dataset {kind} exceeds limit {limit}"),
            Self::Csv { row, field, code } => write!(f, "dataset record {row}, field {field:?}: {code}"),
            Self::Schema { missing, extra, duplicate } => write!(f, "invalid dataset header (missing {missing:?}, extra {extra:?}, duplicate {duplicate:?})"),
            Self::InvalidRow { row, field, code, .. } => write!(f, "dataset record {row}, field {field}: {code}"),
            Self::FamilyConflict { code, .. } => write!(f, "dataset family: {code}"),
            Self::ReleaseCounts { invariant, expected, actual } => write!(f, "dataset release {invariant}: expected {expected}, got {actual}"),
            Self::ChangedDuringRead { .. } => f.write_str("dataset changed during read; retry the operation"),
        }
    }
}
impl std::error::Error for DatasetError {}

fn io_error(path: &Path, operation: &'static str, error: io::Error) -> DatasetError {
    DatasetError::Io {
        path: path.to_path_buf(),
        operation,
        reason: error.kind().to_string(),
    }
}
fn fingerprint(meta: &Metadata) -> FileFingerprint {
    #[cfg(unix)]
    let (changed_ns, device, inode) = {
        use std::os::unix::fs::MetadataExt;
        let changed = u128::try_from(meta.ctime()).ok().and_then(|s| {
            u128::try_from(meta.ctime_nsec())
                .ok()
                .map(|n| s * 1_000_000_000 + n)
        });
        (changed, Some(meta.dev()), Some(meta.ino()))
    };
    #[cfg(not(unix))]
    let (changed_ns, device, inode) = (None, None, None);
    FileFingerprint {
        length: meta.len(),
        modified_ns: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos()),
        changed_ns,
        device,
        inode,
    }
}

/// Resolves one authoritative selection without parsing the CSV contents.
///
/// Explicit/env errors never fall back. Only absent automatic candidates do.
pub fn resolve_dataset(request: &DatasetRequest) -> Result<DatasetResolution, DatasetError> {
    for path in [&request.cwd, &request.executable, &request.home] {
        if !path.is_absolute() {
            return Err(DatasetError::InvalidSelection {
                candidate: DatasetCandidate::Home,
                path: path.clone(),
                reason: "runtime_path_not_absolute".into(),
            });
        }
    }
    let mut searched = Vec::new();
    if let Some(path) = &request.explicit_file {
        return select(
            request,
            DatasetCandidate::Explicit,
            path,
            true,
            &mut searched,
        )?
        .ok_or(DatasetError::NotFound { searched });
    }
    if let Some(path) = &request.environment_file {
        return select(
            request,
            DatasetCandidate::Environment,
            path,
            true,
            &mut searched,
        )?
        .ok_or(DatasetError::NotFound { searched });
    }
    if let Some(path) = &request.configured_file {
        return select(
            request,
            DatasetCandidate::Configured,
            path,
            true,
            &mut searched,
        )?
        .ok_or(DatasetError::NotFound { searched });
    }
    if let Some(root) = workspace_root(request, &mut searched) {
        for (candidate, directory) in [
            (DatasetCandidate::RepoSingular, "dataset"),
            (DatasetCandidate::RepoPlural, "datasets"),
        ] {
            if let Some(found) = select(
                request,
                candidate,
                &root.join(directory).join(DATASET_FILE_NAME),
                false,
                &mut searched,
            )? {
                return Ok(found);
            }
        }
    }
    if let Some(found) = select(
        request,
        DatasetCandidate::Home,
        &request.home.join("dataset").join(DATASET_FILE_NAME),
        false,
        &mut searched,
    )? {
        return Ok(found);
    }
    Err(DatasetError::NotFound { searched })
}

fn select(
    request: &DatasetRequest,
    candidate: DatasetCandidate,
    path: &Path,
    authoritative: bool,
    searched: &mut Vec<CandidateProbe>,
) -> Result<Option<DatasetResolution>, DatasetError> {
    if path.as_os_str().is_empty() {
        return Err(DatasetError::InvalidSelection {
            candidate,
            path: path.to_path_buf(),
            reason: "empty_path".into(),
        });
    }
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        request.cwd.join(path)
    };
    let meta = match fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound && !authoritative => {
            searched.push(CandidateProbe {
                candidate,
                path,
                diagnostic: Some(DatasetDiagnostic {
                    code: "not_found".into(),
                    message: "candidate absent".into(),
                }),
            });
            return Ok(None);
        }
        Err(e) => return Err(io_error(&path, "metadata", e)),
    };
    if !meta.is_file() {
        return Err(DatasetError::InvalidSelection {
            candidate,
            path,
            reason: "not_regular_file".into(),
        });
    }
    if meta.len() > MAX_CSV_BYTES {
        return Err(DatasetError::TooLarge {
            path,
            limit: MAX_CSV_BYTES,
            kind: "file_bytes".into(),
        });
    }
    let file = File::open(&path).map_err(|e| io_error(&path, "open", e))?;
    let opened = file
        .metadata()
        .map_err(|e| io_error(&path, "metadata", e))?;
    if !opened.is_file() || fingerprint(&opened) != fingerprint(&meta) {
        return Err(DatasetError::ChangedDuringRead { path });
    }
    let canonical = fs::canonicalize(&path).map_err(|e| io_error(&path, "canonicalize", e))?;
    searched.push(CandidateProbe {
        candidate,
        path: canonical.clone(),
        diagnostic: None,
    });
    Ok(Some(DatasetResolution {
        candidate,
        path: canonical,
        searched: searched.clone(),
        fingerprint: fingerprint(&opened),
    }))
}

fn workspace_root(request: &DatasetRequest, probes: &mut Vec<CandidateProbe>) -> Option<PathBuf> {
    let mut seen = BTreeSet::new();
    for start in [&request.cwd, &request.executable] {
        let canonical = match fs::canonicalize(start) {
            Ok(path) => path,
            Err(e) => {
                discovery_diagnostic(probes, start, &e.kind().to_string());
                continue;
            }
        };
        let start = if start == &request.executable {
            canonical.parent().unwrap_or(&canonical)
        } else {
            &canonical
        };
        for (depth, root) in start.ancestors().enumerate() {
            if depth >= 64 {
                discovery_diagnostic(probes, root, "ancestor_limit");
                break;
            }
            if !seen.insert(root.to_path_buf()) {
                continue;
            }
            let marker = root.join("Cargo.toml");
            let file = match File::open(&marker) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    discovery_diagnostic(probes, &marker, &e.kind().to_string());
                    continue;
                }
            };
            let mut bytes = Vec::new();
            if let Err(e) = file.take(1024 * 1024 + 1).read_to_end(&mut bytes) {
                discovery_diagnostic(probes, &marker, &e.kind().to_string());
                continue;
            }
            if bytes.len() > 1024 * 1024 {
                discovery_diagnostic(probes, &marker, "marker_too_large");
                continue;
            }
            let parsed = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| s.parse::<toml::Value>().ok());
            let Some(parsed) = parsed else {
                discovery_diagnostic(probes, &marker, "malformed_marker");
                continue;
            };
            let matches = parsed
                .get("workspace")
                .and_then(|v| v.get("members"))
                .and_then(toml::Value::as_array)
                .is_some_and(|members| {
                    members
                        .iter()
                        .any(|v| v.as_str() == Some("crates/argos-osint-core"))
                });
            if matches && root.join("crates/argos-osint-core/Cargo.toml").is_file() {
                return Some(root.to_path_buf());
            }
        }
    }
    None
}
fn discovery_diagnostic(probes: &mut Vec<CandidateProbe>, path: &Path, code: &str) {
    probes.push(CandidateProbe {
        candidate: DatasetCandidate::RepoSingular,
        path: path.to_path_buf(),
        diagnostic: Some(DatasetDiagnostic {
            code: code.into(),
            message: "workspace discovery diagnostic".into(),
        }),
    });
}

/// RFC quote-state scanner bounds raw records before the csv decoder allocates.
/// It also hashes exactly the bytes delivered to that decoder.
struct BoundedCsvReader {
    file: File,
    path: PathBuf,
    total: u64,
    record: usize,
    row: u64,
    state: QuoteState,
    after_cr: bool,
    last_record_bytes: usize,
    prefix: Vec<u8>,
    hash: Sha256,
    failure: Option<DatasetError>,
}
#[derive(Clone, Copy)]
enum QuoteState {
    Start,
    Plain,
    Quoted,
    Closed,
}
impl BoundedCsvReader {
    fn new(file: File, path: PathBuf) -> Self {
        Self {
            file,
            path,
            total: 0,
            record: 0,
            row: 1,
            state: QuoteState::Start,
            after_cr: false,
            last_record_bytes: 0,
            prefix: Vec::with_capacity(3),
            hash: Sha256::new(),
            failure: None,
        }
    }
    fn inspect(&mut self, byte: u8) -> Result<(), DatasetError> {
        self.total += 1;
        if self.total > MAX_CSV_BYTES {
            return Err(DatasetError::TooLarge {
                path: self.path.clone(),
                limit: MAX_CSV_BYTES,
                kind: "file_bytes".into(),
            });
        }
        self.record += 1;
        if self.record > MAX_CSV_RECORD_BYTES {
            return Err(DatasetError::TooLarge {
                path: self.path.clone(),
                limit: MAX_CSV_RECORD_BYTES as u64,
                kind: "record_bytes".into(),
            });
        }
        // Keep the BOM out of quote state, but include it in both bounds/hash.
        if self.total <= 3 {
            self.prefix.push(byte);
            if self.prefix == b"\xef" || self.prefix == b"\xef\xbb" {
                return Ok(());
            }
            if self.prefix == b"\xef\xbb\xbf" {
                return Ok(());
            }
        }
        if self.after_cr {
            self.after_cr = false;
            if byte == b'\n' {
                if self.last_record_bytes + 1 > MAX_CSV_RECORD_BYTES {
                    return Err(DatasetError::TooLarge {
                        path: self.path.clone(),
                        limit: MAX_CSV_RECORD_BYTES as u64,
                        kind: "record_bytes".into(),
                    });
                }
                self.record = 0;
                return Ok(());
            }
        }
        match self.state {
            QuoteState::Quoted => {
                if byte == b'"' {
                    self.state = QuoteState::Closed;
                }
            }
            QuoteState::Closed => match byte {
                b'"' => self.state = QuoteState::Quoted,
                b',' => self.state = QuoteState::Start,
                b'\r' | b'\n' => self.end_record(byte),
                _ => {
                    return Err(DatasetError::Csv {
                        row: self.row,
                        field: None,
                        code: "malformed_csv".into(),
                    })
                }
            },
            QuoteState::Start => match byte {
                b'"' => self.state = QuoteState::Quoted,
                b',' => {}
                b'\r' | b'\n' => self.end_record(byte),
                _ => self.state = QuoteState::Plain,
            },
            QuoteState::Plain => match byte {
                b'"' => {
                    return Err(DatasetError::Csv {
                        row: self.row,
                        field: None,
                        code: "malformed_csv".into(),
                    })
                }
                b',' => self.state = QuoteState::Start,
                b'\r' | b'\n' => self.end_record(byte),
                _ => {}
            },
        }
        Ok(())
    }
    fn end_record(&mut self, byte: u8) {
        self.state = QuoteState::Start;
        self.last_record_bytes = self.record;
        self.record = 0;
        self.row += 1;
        self.after_cr = byte == b'\r';
    }
}
impl Read for BoundedCsvReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.failure.is_some() {
            return Err(io::Error::other("dataset boundary rejected"));
        }
        let n = self.file.read(output)?;
        if n == 0 && matches!(self.state, QuoteState::Quoted) {
            self.failure = Some(DatasetError::Csv {
                row: self.row,
                field: None,
                code: "malformed_csv".into(),
            });
            return Err(io::Error::other("unterminated CSV quote"));
        }
        for byte in &output[..n] {
            if let Err(error) = self.inspect(*byte) {
                self.failure = Some(error);
                return Err(io::Error::other("dataset boundary rejected"));
            }
        }
        self.hash.update(&output[..n]);
        Ok(n)
    }
}

/// Parses and validates a selected synthetic CSV without changing any file.
///
/// The same open stream supplies hashing and RFC decoding. Content changes
/// observed before/after reading abort the complete operation.
pub fn parse_csv(resolution: &DatasetResolution) -> Result<ParsedDataset, DatasetError> {
    let file = File::open(&resolution.path).map_err(|e| io_error(&resolution.path, "open", e))?;
    let before = file
        .metadata()
        .map_err(|e| io_error(&resolution.path, "metadata", e))?;
    if !before.is_file() || fingerprint(&before) != resolution.fingerprint {
        return Err(DatasetError::ChangedDuringRead {
            path: resolution.path.clone(),
        });
    }
    let source = BoundedCsvReader::new(file, resolution.path.clone());
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(source);
    let result = decode_csv(&mut reader, resolution);
    let source = reader.into_inner();
    if let Some(failure) = source.failure {
        return Err(failure);
    }
    let after = source
        .file
        .metadata()
        .map_err(|e| io_error(&resolution.path, "metadata", e))?;
    let path_after =
        fs::metadata(&resolution.path).map_err(|e| io_error(&resolution.path, "metadata", e))?;
    if fingerprint(&before) != fingerprint(&after)
        || fingerprint(&after) != fingerprint(&path_after)
    {
        return Err(DatasetError::ChangedDuringRead {
            path: resolution.path.clone(),
        });
    }
    let (rows, version, summary) = result?;
    let hash = format!("{:x}", source.hash.finalize());
    let provenance = Provenance::synthetic_baseline(&version, &hash);
    let rows: Vec<_> = rows
        .into_iter()
        .map(|mut row| {
            row.provenance = provenance.clone();
            row
        })
        .collect();
    let hashes =
        training::corpus_hashes(&rows).map_err(|error| map_training_error(error, &rows))?;
    let corpus = TrainingCorpus {
        rows,
        dataset_sha256: hash.clone(),
        train_split_sha256: hashes.train_split_sha256,
        calibration_split_sha256: hashes.calibration_split_sha256,
        development_split_sha256: hashes.development_split_sha256,
        test_split_sha256: hashes.test_split_sha256,
        family_assignment_sha256: hashes.family_assignment_sha256,
    };
    training::validate_corpus(&corpus).map_err(|error| map_training_error(error, &corpus.rows))?;
    Ok(ParsedDataset {
        resolution: resolution.clone(),
        profile: if version == SEED_DATASET_VERSION {
            CsvProfile::DeliveredSeedV1
        } else if version == DISTINCT_DATASET_VERSION {
            CsvProfile::DeliveredDistinctV2
        } else {
            CsvProfile::SyntheticDevelopment
        },
        dataset_version: version,
        corpus,
        summary: DatasetSummary {
            dataset_sha256: hash,
            ..summary
        },
    })
}

fn map_training_error(error: ClassifierError, rows: &[DatasetRow]) -> DatasetError {
    match error {
        ClassifierError::InvalidTrainingData { row_id, reason } => DatasetError::InvalidRow {
            row: row_id
                .as_ref()
                .and_then(|id| rows.iter().position(|r| &r.sample_id == id))
                .map_or(2, |i| i as u64 + 2),
            sample_id: row_id,
            field: "corpus".into(),
            code: reason,
        },
        _ => DatasetError::InvalidRow {
            row: 2,
            sample_id: None,
            field: "corpus".into(),
            code: "invalid_training_corpus".into(),
        },
    }
}
fn csv_error(error: csv::Error, row: u64, path: &Path) -> DatasetError {
    if let csv::ErrorKind::Io(error) = error.kind() {
        return DatasetError::Io {
            path: path.to_path_buf(),
            operation: "read",
            reason: error.kind().to_string(),
        };
    }
    let code = if matches!(error.kind(), csv::ErrorKind::Utf8 { .. }) {
        "invalid_utf8"
    } else {
        "malformed_csv"
    };
    DatasetError::Csv {
        row,
        field: None,
        code: code.into(),
    }
}

#[derive(Debug)]
struct Family {
    split: usize,
    membership: [LabelMembership; 6],
    variants: u64,
}
fn decode_csv(
    reader: &mut csv::Reader<BoundedCsvReader>,
    resolution: &DatasetResolution,
) -> Result<(Vec<DatasetRow>, String, DatasetSummary), DatasetError> {
    let headers = reader
        .headers()
        .map_err(|e| csv_error(e, 1, &resolution.path))?
        .clone();
    let mut positions = BTreeMap::new();
    let mut duplicate = Vec::new();
    let mut extra = Vec::new();
    // csv-core consumes one leading BOM. A second is an unknown header.
    for (index, value) in headers.iter().enumerate() {
        if positions.insert(value.to_string(), index).is_some() {
            duplicate.push(value.into());
        }
        if !HEADER.contains(&value) {
            extra.push(value.into());
        }
    }
    let missing: Vec<_> = HEADER
        .iter()
        .filter(|field| !positions.contains_key(**field))
        .map(|s| (*s).to_string())
        .collect();
    if !missing.is_empty() || !extra.is_empty() || !duplicate.is_empty() {
        return Err(DatasetError::Schema {
            missing,
            extra,
            duplicate,
        });
    }
    let order: Vec<usize> = HEADER.iter().map(|field| positions[*field]).collect();
    let mut rows = Vec::new();
    let mut version = String::new();
    let mut ids = BTreeSet::new();
    let mut titles = BTreeSet::new();
    let mut descriptions = BTreeSet::new();
    let mut families: BTreeMap<String, Family> = BTreeMap::new();
    let mut summary = DatasetSummary {
        rows: 0,
        families: 0,
        split_rows: [0; 4],
        split_families: [0; 4],
        positive_rows: [[0; 6]; 4],
        dataset_sha256: String::new(),
    };
    let mut text_bytes = 0;
    let mut record = csv::StringRecord::new();
    while reader
        .read_record(&mut record)
        .map_err(|e| csv_error(e, summary.rows + 2, &resolution.path))?
    {
        let number = summary.rows + 2;
        if record.len() != 18 {
            return Err(DatasetError::Csv {
                row: number,
                field: None,
                code: "field_count".into(),
            });
        }
        let values: Vec<_> = order.iter().map(|index| &record[*index]).collect();
        let invalid = |field: &str, code: &str| DatasetError::InvalidRow {
            row: number,
            sample_id: None,
            field: field.into(),
            code: code.into(),
        };
        for (i, value) in values.iter().enumerate() {
            if i != 1 && i != 2 && value.len() > MAX_METADATA_FIELD_BYTES {
                return Err(DatasetError::TooLarge {
                    path: resolution.path.clone(),
                    limit: MAX_METADATA_FIELD_BYTES as u64,
                    kind: format!("field_{}", HEADER[i]),
                });
            }
        }
        for index in [0, 9, 12] {
            if values[index].is_empty()
                || values[index].len() > 256
                || values[index].chars().any(char::is_control)
            {
                return Err(invalid(HEADER[index], "invalid_identifier"));
            }
        }
        if !ids.insert(values[0].to_string()) {
            return Err(invalid("sample_id", "duplicate_id"));
        }
        if version.is_empty() {
            version = values[9].into();
        }
        if values[9] != version {
            return Err(invalid("dataset_version", "mixed_dataset_version"));
        }
        let release = version == SEED_DATASET_VERSION;
        let distinct = version == DISTINCT_DATASET_VERSION;
        // Small independently authored corpora have explicit schema/policy versions.
        // Unknown versions cannot inherit an admission policy by accident.
        if !matches!(
            version.as_str(),
            SEED_DATASET_VERSION
                | DISTINCT_DATASET_VERSION
                | "synthetic-development-v1"
                | "synthetic-development-v2"
        ) {
            return Err(invalid("dataset_version", "unsupported_dataset_version"));
        }
        let modern = distinct || version == "synthetic-development-v2";
        if !descriptions.insert(values[2].trim().to_string()) && distinct {
            return Err(invalid("description", "duplicate_description"));
        }
        if !titles.insert(values[1].trim().to_string()) && (release || distinct) {
            return Err(invalid("title", "duplicate_title"));
        }
        if values[1].trim().is_empty() || values[2].trim().is_empty() {
            return Err(invalid("title/description", "blank_text"));
        }
        let combined = values[1].len() + values[2].len();
        if combined > training::MAX_ROW_TEXT_BYTES {
            return Err(DatasetError::TooLarge {
                path: resolution.path.clone(),
                limit: training::MAX_ROW_TEXT_BYTES as u64,
                kind: "row_text_bytes".into(),
            });
        }
        text_bytes += combined;
        if text_bytes > training::MAX_TRAINING_TEXT_BYTES {
            return Err(DatasetError::TooLarge {
                path: resolution.path.clone(),
                limit: training::MAX_TRAINING_TEXT_BYTES as u64,
                kind: "corpus_text_bytes".into(),
            });
        }
        if rows.len() == training::MAX_TRAINING_ROWS {
            return Err(DatasetError::TooLarge {
                path: resolution.path.clone(),
                limit: training::MAX_TRAINING_ROWS as u64,
                kind: "rows".into(),
            });
        }
        if values[7] != "synthetic" {
            return Err(invalid("source_kind", "unsupported_provenance"));
        }
        for index in [14, 15] {
            if values[index] != "true" {
                return Err(invalid(HEADER[index], "invalid_boolean"));
            }
        }
        if (release && values[13] != "en")
            || (!release
                && values[13] != "en"
                && !(values[13].starts_with("en-") && values[13].len() > 3))
        {
            return Err(invalid("language", "unsupported_language"));
        }
        if release && !values[6].is_empty() {
            return Err(invalid("subcategories", "annotation_mismatch"));
        }
        let (split, split_index) = match values[11] {
            "train" => (Split::Train, 0),
            "calibration" => (Split::Calibration, 1),
            "development" if modern => (Split::Development, 2),
            "test" => (Split::Test, 3),
            _ => return Err(invalid("split", "unsupported_split")),
        };
        let mut labels = BTreeSet::new();
        if !values[3].is_empty() {
            for token in values[3].split('|') {
                let domain = DomainId::parse(token)
                    .ok_or_else(|| invalid("domain_labels", "unknown_domain"))?;
                if !labels.insert(domain) {
                    return Err(invalid("domain_labels", "duplicate_domain"));
                }
            }
        }
        if labels.len() > 2 && !modern {
            return Err(invalid("domain_labels", "annotation_mismatch"));
        }
        let editorial: BTreeSet<_> = [values[4], values[5]]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let expected: BTreeSet<_> = labels.iter().map(|d| d.as_str().to_string()).collect();
        if (labels.len() <= 2 && editorial != expected)
            || (labels.len() > 2
                && (editorial.len() != 2
                    || !editorial.is_subset(&expected)
                    || values[4].is_empty()
                    || values[5].is_empty()))
            || (labels.is_empty() && (!values[4].is_empty() || !values[5].is_empty()))
            || (labels.len() == 1 && (values[4].is_empty() || !values[5].is_empty()))
            || (labels.len() == 2
                && (values[4] == values[5] || values[4].is_empty() || values[5].is_empty()))
        {
            return Err(invalid(
                "primary_domain/secondary_domain",
                "annotation_mismatch",
            ));
        }
        let (admission, reason, valid_difficulty) = match labels.len() {
            0 => (
                "reject",
                "out_of_scope",
                matches!(values[8], "out_of_scope" | "hard_negative"),
            ),
            1 if !modern => (
                "reject",
                "insufficient_domains",
                values[8] == "single_domain_boundary",
            ),
            1 => ("admit", "", values[8] == "single_domain_boundary"),
            2 => ("admit", "", values[8] == "cross_domain"),
            _ => ("admit", "", values[8] == "multi_domain"),
        };
        if values[16] != admission || values[17] != reason || !valid_difficulty {
            return Err(invalid(
                "expected_admission/rejection_reason/difficulty",
                "annotation_mismatch",
            ));
        }
        let membership = DomainId::ALL.map(|domain| {
            if labels.contains(&domain) {
                LabelMembership::Present
            } else {
                LabelMembership::Absent
            }
        });
        match families.get_mut(values[12]) {
            Some(family) => {
                if family.split != split_index {
                    return Err(DatasetError::FamilyConflict {
                        family_id: values[12].into(),
                        code: "family_split_leakage".into(),
                    });
                }
                if family.membership != membership {
                    return Err(DatasetError::FamilyConflict {
                        family_id: values[12].into(),
                        code: "family_label_conflict".into(),
                    });
                }
                family.variants += 1;
            }
            None => {
                families.insert(
                    values[12].into(),
                    Family {
                        split: split_index,
                        membership,
                        variants: 1,
                    },
                );
                summary.split_families[split_index] += 1;
            }
        }
        summary.rows += 1;
        summary.split_rows[split_index] += 1;
        for (i, domain) in DomainId::ALL.iter().enumerate() {
            if labels.contains(domain) {
                summary.positive_rows[split_index][i] += 1;
            }
        }
        rows.push(DatasetRow {
            sample_id: values[0].into(),
            family_id: FamilyId::parse(values[12])
                .map_err(|_| invalid("scenario_group", "invalid_identifier"))?,
            split,
            title: values[1].into(),
            description: values[2].into(),
            language: values[13].into(),
            membership,
            labels_complete: true,
            provenance: Provenance::synthetic_baseline(&version, "pending"),
        });
    }
    if rows.is_empty() {
        return Err(DatasetError::InvalidRow {
            row: 2,
            sample_id: None,
            field: "corpus".into(),
            code: "empty_corpus".into(),
        });
    }
    summary.families = families.len() as u64;
    if version == SEED_DATASET_VERSION {
        validate_release(&summary, &families)?;
    } else if version == DISTINCT_DATASET_VERSION {
        validate_distinct_release(&summary, &families)?;
    }
    Ok((rows, version, summary))
}

fn release_count(invariant: &str, expected: u64, actual: u64) -> Result<(), DatasetError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DatasetError::ReleaseCounts {
            invariant: invariant.into(),
            expected,
            actual,
        })
    }
}
fn validate_release(
    summary: &DatasetSummary,
    families: &BTreeMap<String, Family>,
) -> Result<(), DatasetError> {
    release_count("rows", 1700, summary.rows)?;
    release_count("families", 220, summary.families)?;
    for (i, expected) in [1130, 285, 0, 285].into_iter().enumerate() {
        release_count(&format!("split_{i}_rows"), expected, summary.split_rows[i])?;
    }
    for (i, expected) in [142, 39, 0, 39].into_iter().enumerate() {
        release_count(
            &format!("split_{i}_families"),
            expected,
            summary.split_families[i],
        )?;
    }
    for (i, expected) in [190, 55, 0, 55].into_iter().enumerate() {
        for (d, actual) in summary.positive_rows[i].iter().enumerate() {
            release_count(
                &format!("split_{i}_domain_{d}_positives"),
                expected,
                *actual,
            )?;
        }
    }
    let mut focused = [0; 6];
    let mut pairs: BTreeMap<(usize, usize), [u64; 4]> = BTreeMap::new();
    let mut negatives = 0;
    for family in families.values() {
        let positive: Vec<_> = family
            .membership
            .iter()
            .enumerate()
            .filter(|(_, m)| **m == LabelMembership::Present)
            .map(|(i, _)| i)
            .collect();
        match positive.as_slice() {
            [] => {
                release_count("negative_family_variants", 5, family.variants)?;
                negatives += 1;
            }
            [one] => {
                release_count("focused_family_variants", 10, family.variants)?;
                focused[*one] += 1;
            }
            [one, two] => {
                release_count("overlap_family_variants", 5, family.variants)?;
                pairs.entry((*one, *two)).or_default()[family.split] += 1;
            }
            _ => {
                return Err(DatasetError::ReleaseCounts {
                    invariant: "family_domain_count".into(),
                    expected: 2,
                    actual: positive.len() as u64,
                })
            }
        }
    }
    release_count("negative_families", 40, negatives)?;
    for (i, count) in focused.into_iter().enumerate() {
        release_count(&format!("domain_{i}_focused_families"), 20, count)?;
    }
    for one in 0..6 {
        for two in one + 1..6 {
            let counts = pairs.get(&(one, two)).copied().unwrap_or([0; 4]);
            release_count(
                &format!("pair_{one}_{two}_families"),
                4,
                counts.iter().sum(),
            )?;
            for (split, count) in counts.iter().enumerate() {
                if split != 2 && *count == 0 {
                    return Err(DatasetError::ReleaseCounts {
                        invariant: format!("pair_{one}_{two}_split_{split}_presence"),
                        expected: 1,
                        actual: 0,
                    });
                }
            }
        }
    }
    Ok(())
}

/// The delivered v2 corpus is one row per independently authored scenario.
fn validate_distinct_release(
    summary: &DatasetSummary,
    families: &BTreeMap<String, Family>,
) -> Result<(), DatasetError> {
    release_count("rows", 1020, summary.rows)?;
    release_count("families", 1020, summary.families)?;
    for (split, expected) in [440, 230, 165, 185].into_iter().enumerate() {
        release_count(
            &format!("split_{split}_rows"),
            expected,
            summary.split_rows[split],
        )?;
        release_count(
            &format!("split_{split}_families"),
            expected,
            summary.split_families[split],
        )?;
        for (domain, actual) in summary.positive_rows[split].iter().enumerate() {
            release_count(
                &format!("split_{split}_domain_{domain}_positives"),
                [80, 40, 30, 40][split],
                *actual,
            )?;
        }
    }
    let mut counts: BTreeMap<Vec<usize>, [u64; 4]> = BTreeMap::new();
    for family in families.values() {
        release_count("scenario_variants", 1, family.variants)?;
        let labels: Vec<_> = family
            .membership
            .iter()
            .enumerate()
            .filter_map(|(index, label)| (*label == LabelMembership::Present).then_some(index))
            .collect();
        counts.entry(labels).or_default()[family.split] += 1;
    }
    let check = |labels: Vec<usize>, expected: [u64; 4]| -> Result<(), DatasetError> {
        let actual = counts.get(&labels).copied().unwrap_or_default();
        for split in 0..4 {
            release_count(
                &format!("labels_{labels:?}_split_{split}"),
                expected[split],
                actual[split],
            )?;
        }
        Ok(())
    };
    check(vec![], [50, 20, 15, 15])?;
    for first in 0..6 {
        check(vec![first], [50, 30, 20, 20])?;
        for second in first + 1..6 {
            check(vec![first, second], [6, 2, 2, 2])?;
            for third in second + 1..6 {
                check(vec![first, second, third], [0, 0, 0, 1])?;
            }
        }
    }
    Ok(())
}
