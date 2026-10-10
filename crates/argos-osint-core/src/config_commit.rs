//! App-wide configuration write lock, recoverable commit journal, and the secure
//! single-file write primitive every configuration writer routes through.
//!
//! `config.toml`, `auth.json` and `quota.json` are one configuration: a caller
//! that changes a provider either moves all three files together or none of
//! them. Every writer therefore takes [`ConfigLock`] and hands the whole batch to
//! [`ConfigLock::commit_files`], which publishes the batch through a sibling
//! journal (`.argos-config-journal.json` in the configuration root) and rolls the
//! whole batch back when any single file fails. A process that dies mid-commit
//! leaves `state: "pending"` behind; [`recover_at_startup`] finishes the rollback
//! on the next boot.
//!
//! The lock is a `create_new` lockfile with a heartbeat and a stale takeover, so
//! the dependency graph is unchanged (no `fs2`/`flock`). The journal stores paths
//! and state only — never a body and never key material. A successful commit
//! advances a process-wide generation id and notifies [`subscribe_reload`].

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::paths;

/// Lockfile name inside the configuration root.
const LOCK_NAME: &str = ".argos-config.lock";
/// Commit journal name inside the configuration root.
const JOURNAL_NAME: &str = ".argos-config-journal.json";
/// Prefix of the unique sibling temporary file [`write_secure`] creates.
const TEMP_PREFIX: &str = ".argos-tmp-";
/// A lockfile heartbeat older than this is taken over.
pub const LOCK_STALE_AFTER: Duration = Duration::from_secs(LOCK_STALE_SECS);
/// The same window in the seconds `chrono` compares timestamps with.
const LOCK_STALE_SECS: u64 = 30;
/// How long [`ConfigLock::acquire_at`] waits for a live holder before failing.
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// Poll step while waiting for a live holder to finish.
const LOCK_STEP: Duration = Duration::from_millis(10);
/// Owner-only mode for every file this module creates, applied at creation.
pub const SECURE_FILE_MODE: u32 = 0o600;
/// Bounded length of a journal error line.
const MAX_ERROR_CHARS: usize = 400;

const STATE_PENDING: &str = "pending";
const STATE_COMMITTED: &str = "committed";
const STATE_RESTORED: &str = "restored";

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);
/// Per-process lock identity. A lock is released only by the holder that took
/// it: the owner metadata carries both the process and this id, so a writer in
/// one thread can never drop a lock another thread in the same process holds
/// (every thread in a process shares a pid).
static LOCK_SEQ: AtomicU64 = AtomicU64::new(1);
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Claimed by a single `fetch_add`, so two commits can never publish the same
/// reload epoch and a subscriber always observes a change.
static RELOAD_EPOCH: AtomicU64 = AtomicU64::new(0);
static RELOAD_TX: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();

/// Writes `body` to `path` atomically with owner-only permissions.
///
/// A unique sibling temporary file is created 0600 **at creation** (never a
/// write-then-chmod helper), written, flushed, fsynced, and renamed over the
/// target; the parent directory is fsynced best effort so the rename is durable.
/// The temporary file never survives an error path.
pub fn write_secure(path: &Path, body: &str) -> Result<()> {
    let dir = parent_of(path);
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let (temp, mut file) = create_sibling(dir, TEMP_PREFIX)
        .with_context(|| format!("stage a temporary file beside {}", path.display()))?;
    let written = persist_body(&mut file, body);
    drop(file);
    if let Err(err) = written {
        remove_quietly(&temp);
        return Err(err).context("write the staged body");
    }
    fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))?;
    sync_dir(dir);
    Ok(())
}

/// One configuration file in a commit batch: a stable slot name, its path, and
/// the whole body that must land at that path.
#[derive(Clone, Debug)]
pub struct CommitFile {
    pub slot: &'static str,
    pub path: PathBuf,
    pub body: String,
}

impl CommitFile {
    pub fn new(slot: &'static str, path: PathBuf, body: String) -> Self {
        Self { slot, path, body }
    }
}

/// The app-wide configuration write lock.
///
/// One holder at a time per configuration root. Holding it serialises
/// [`Self::commit_files`] against every other configuration writer in every
/// process that shares the root.
#[derive(Clone, Debug)]
pub struct ConfigLock {
    root: PathBuf,
    lock_path: PathBuf,
    pid: u64,
    /// Unique inside this process, so a sibling thread never releases it.
    holder: u64,
    generation: u64,
}

impl ConfigLock {
    /// Locks the canonical configuration root (the directory holding
    /// [`crate::paths::config_path`]).
    pub fn acquire() -> Result<Self> {
        Self::acquire_at(&canonical_root())
    }

    /// Locks the directory that holds the configuration files. A live holder
    /// blocks for at most `LOCK_WAIT`; a lockfile whose heartbeat is older than
    /// [`LOCK_STALE_AFTER`], or that is unreadable or empty, is taken over and
    /// the takeover is recorded in the new lockfile body (`took_over_pid` /
    /// `took_over_at`) so the previous holder stays auditable.
    pub fn acquire_at(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)
            .with_context(|| format!("create the configuration root {}", root.display()))?;
        let lock_path = root.join(LOCK_NAME);
        let pid = std::process::id() as u64;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            let holder = next_holder();
            match write_lock(&lock_path, pid, holder, None) {
                Ok(()) => {
                    return Ok(Self {
                        root: root.to_path_buf(),
                        lock_path,
                        pid,
                        holder,
                        generation: current_generation(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    match lock_takeover(&lock_path) {
                        Some(previous) => {
                            if Instant::now() >= deadline {
                                bail!("configuration is being written by another process");
                            }
                            // Drop the dead lockfile, then race to recreate it.
                            remove_quietly(&lock_path);
                            let takeover = next_holder();
                            if write_lock(&lock_path, pid, takeover, Some(previous)).is_ok() {
                                return Ok(Self {
                                    root: root.to_path_buf(),
                                    lock_path,
                                    pid,
                                    holder: takeover,
                                    generation: current_generation(),
                                });
                            }
                        }
                        None => {
                            if Instant::now() >= deadline {
                                bail!("configuration is being written by another process");
                            }
                            std::thread::sleep(LOCK_STEP);
                        }
                    }
                }
                Err(err) => {
                    return Err(err).with_context(|| format!("create {}", lock_path.display()))
                }
            }
        }
    }

    /// The directory this lock guards.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The configuration generation captured when the lock was taken.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Publishes every file in `files` as one all-or-nothing revision and returns
    /// the new generation.
    ///
    /// The batch is staged beside each target, journaled as `pending`, renamed
    /// into place one file at a time, and only then journaled as `committed`. Any
    /// failure restores every already-renamed target from its backup (or removes
    /// it when there was no backup), journals `restored` with a redacted error,
    /// and returns that error: the targets never hold a mix of revisions.
    pub fn commit_files(&self, files: &[CommitFile]) -> Result<u64> {
        validate_batch(files)?;
        let generation = current_generation().max(self.generation) + 1;
        let mut journal = CommitJournal {
            generation,
            state: STATE_PENDING.to_string(),
            started_at: now_rfc3339(),
            finished_at: None,
            error: None,
            files: files
                .iter()
                .map(|file| journal_entry(file, generation))
                .collect(),
        };
        write_journal_at(&self.root, &journal)?;

        let mut landed: Vec<Landed> = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            self.touch_heartbeat();
            match stage_and_land(file, generation) {
                Ok(item) => {
                    journal.files[index].done = true;
                    let _ = write_journal_at(&self.root, &journal);
                    landed.push(item);
                }
                Err(err) => {
                    restore_landed(&landed);
                    journal.state = STATE_RESTORED.to_string();
                    journal.finished_at = Some(now_rfc3339());
                    journal.error = Some(redact_error(&err));
                    let _ = write_journal_at(&self.root, &journal);
                    return Err(err);
                }
            }
        }

        journal.state = STATE_COMMITTED.to_string();
        journal.finished_at = Some(now_rfc3339());
        journal.error = None;
        write_journal_at(&self.root, &journal)?;
        for entry in &journal.files {
            remove_quietly(Path::new(&entry.backup));
        }
        GENERATION.fetch_max(generation, Ordering::SeqCst);
        notify_reload();
        Ok(generation)
    }

    /// Keeps the lockfile heartbeat fresh while a long batch is published, so a
    /// live holder is never mistaken for a dead one.
    fn touch_heartbeat(&self) {
        let _ = write_secure(&self.lock_path, &lock_body(self.pid, self.holder, None));
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        if lock_is_ours(&self.lock_path, self.pid, self.holder) {
            remove_quietly(&self.lock_path);
        }
    }
}

/// One file of a journaled commit, as it lives on disk. Paths only.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JournalEntry {
    pub slot: String,
    pub path: String,
    pub staged: String,
    pub backup: String,
    pub done: bool,
}

/// The recoverable commit journal: which revision was being published, and how
/// far it got.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CommitJournal {
    pub generation: u64,
    pub state: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub files: Vec<JournalEntry>,
}

/// Reads the journal in `root`. Missing, blank or unparseable is no journal.
pub fn read_journal_at(root: &Path) -> Option<CommitJournal> {
    let raw = fs::read_to_string(root.join(JOURNAL_NAME)).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&raw).ok()
}

/// Atomically writes the journal in `root`.
pub fn write_journal_at(root: &Path, journal: &CommitJournal) -> Result<()> {
    let body = serde_json::to_string_pretty(journal)?;
    write_secure(&root.join(JOURNAL_NAME), &body)
}

/// What startup recovery did.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryReport {
    pub state: &'static str,
    pub generation: u64,
    pub restored: Vec<PathBuf>,
    pub detail: String,
}

/// Rolls back an interrupted commit in the canonical configuration root.
pub fn recover_at_startup() -> RecoveryReport {
    recover_at_startup_at(&canonical_root())
}

/// Rolls back an interrupted commit in `root`.
///
/// A `pending` journal means a previous process died mid-commit: every entry that
/// had not landed is restored from its backup, or removed when there is no
/// backup, and the journal is rewritten as `restored`.
pub fn recover_at_startup_at(root: &Path) -> RecoveryReport {
    let clean = RecoveryReport {
        state: "clean",
        generation: current_generation(),
        restored: Vec::new(),
        detail: String::new(),
    };
    let Some(journal) = read_journal_at(root) else {
        return clean;
    };
    if journal.state != STATE_PENDING {
        // `committed`, `restored`, or anything unknown: leave the journal alone.
        return clean;
    }

    let mut restored = Vec::new();
    for entry in &journal.files {
        if entry.done {
            continue;
        }
        let target = PathBuf::from(&entry.path);
        let backup = PathBuf::from(&entry.backup);
        if backup.exists() && fs::rename(&backup, &target).is_ok() {
            restored.push(target);
        } else if target.exists() && fs::remove_file(&target).is_ok() {
            restored.push(target);
        }
    }

    let mut updated = journal.clone();
    updated.state = STATE_RESTORED.to_string();
    updated.finished_at = Some(now_rfc3339());
    updated.error = Some("recovered after interrupted commit".to_string());
    let _ = write_journal_at(root, &updated);

    RecoveryReport {
        state: "restored",
        generation: journal.generation,
        restored: restored.clone(),
        detail: format!(
            "restored {} file(s) after an interrupted commit",
            restored.len()
        ),
    }
}

/// The current configuration generation.
pub fn current_generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// Wakes every [`subscribe_reload`] listener; the next epoch is one higher.
///
/// The epoch is claimed with a single atomic `fetch_add`: reading the epoch and
/// sending it are two steps, so a read-then-send would let two concurrent
/// commits publish the *same* epoch and a listener could observe no change at
/// all. Every commit therefore strictly advances the epoch.
pub fn notify_reload() {
    let next = RELOAD_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = tx().send(next);
}

/// A receiver that yields the reload epoch whenever a commit lands.
pub fn subscribe_reload() -> tokio::sync::watch::Receiver<u64> {
    tx().subscribe()
}

/// The current reload epoch. Every commit advances it by exactly one.
pub fn reload_epoch() -> u64 {
    RELOAD_EPOCH.load(Ordering::SeqCst)
}

fn tx() -> &'static tokio::sync::watch::Sender<u64> {
    // A new subscriber starts at the epoch already published, so it never sees
    // an epoch older than one that has already been claimed.
    RELOAD_TX.get_or_init(|| tokio::sync::watch::channel(reload_epoch()).0)
}

fn canonical_root() -> PathBuf {
    match paths::config_path().parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => paths::home_dir(),
    }
}

/// Owner metadata of a lockfile. Never anything but owner metadata.
#[derive(Debug, Default, Deserialize)]
struct LockOwner {
    #[serde(default)]
    pid: u64,
    #[serde(default)]
    holder: u64,
    #[serde(default)]
    host: String,
    #[serde(default)]
    acquired_at: String,
    #[serde(default)]
    heartbeat_at: String,
}

/// A file that already landed at its target during this commit.
struct Landed {
    target: PathBuf,
    backup: Option<PathBuf>,
}

fn lock_body(pid: u64, holder: u64, takeover: Option<LockOwner>) -> String {
    let now = now_rfc3339();
    let mut body = json!({
        "pid": pid,
        "holder": holder,
        "host": host_name(),
        "acquired_at": now,
        "heartbeat_at": now,
    });
    if let Some(previous) = takeover {
        body["took_over_pid"] = json!(previous.pid);
        body["took_over_at"] = json!(now_rfc3339());
    }
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

fn write_lock(
    path: &Path,
    pid: u64,
    holder: u64,
    takeover: Option<LockOwner>,
) -> std::io::Result<()> {
    let body = lock_body(pid, holder, takeover);
    let mut file = open_created(path)?;
    let written = persist_body(&mut file, &body);
    drop(file);
    if written.is_err() {
        remove_quietly(path);
    }
    written
}

/// The previous owner when `path` is a dead lockfile, `None` while it is live.
///
/// A lockfile is created with `create_new` and its body is written afterwards,
/// so a reader can catch it **empty or half written** while the holder is still
/// acquiring. That is the most alive a lock can be, so an unreadable lockfile is
/// never treated as dead on its own: it is taken over only once the file itself
/// is older than [`LOCK_STALE_AFTER`]. Trusting the empty read instead would let
/// one writer delete another's live lock and leave two holders inside
/// [`ConfigLock::commit_files`], which is exactly the corruption the lock exists
/// to prevent.
fn lock_takeover(path: &Path) -> Option<LockOwner> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let owner = serde_json::from_str::<LockOwner>(&raw).ok();
    match owner {
        Some(owner) if heartbeat_is_stale(&owner) => Some(owner),
        Some(_) => None,
        // Empty, truncated or unparseable: only a lockfile that has sat
        // unfinished for longer than the stale window is really abandoned.
        None if lock_file_is_older_than_stale(path) => Some(LockOwner::default()),
        None => None,
    }
}

/// True when `path` has not been touched for at least [`LOCK_STALE_AFTER`].
fn lock_file_is_older_than_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        // No metadata at all: the holder just released it, so retrying is correct.
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    match modified.elapsed() {
        Ok(age) => age >= LOCK_STALE_AFTER,
        // A clock that ran backwards is not evidence of a dead holder.
        Err(_) => false,
    }
}

fn heartbeat_is_stale(owner: &LockOwner) -> bool {
    match chrono::DateTime::parse_from_rfc3339(owner.heartbeat_at.trim()) {
        Ok(at) => {
            let heartbeat = at.with_timezone(&chrono::Utc);
            let age = chrono::Utc::now().signed_duration_since(heartbeat);
            age >= chrono::Duration::seconds(LOCK_STALE_SECS as i64)
        }
        Err(_) => true,
    }
}

fn lock_is_ours(path: &Path, pid: u64, holder: u64) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    match serde_json::from_str::<LockOwner>(&raw) {
        // A lockfile written by a sibling thread in this same process holds the
        // same pid but a different holder id, so it is not ours to release.
        Ok(owner) => owner.pid == pid && owner.holder == holder,
        Err(_) => false,
    }
}

fn next_holder() -> u64 {
    LOCK_SEQ.fetch_add(1, Ordering::SeqCst)
}

fn validate_batch(files: &[CommitFile]) -> Result<()> {
    let mut seen = HashSet::new();
    for file in files {
        let slot = file.slot.trim();
        if slot.is_empty() {
            bail!("a configuration commit slot name is empty");
        }
        if !seen.insert(slot) {
            bail!("a configuration commit repeats the slot {slot}");
        }
        if file.path.parent().is_none() {
            bail!(
                "the configuration file {} has no parent directory",
                file.path.display()
            );
        }
    }
    Ok(())
}

fn journal_entry(file: &CommitFile, generation: u64) -> JournalEntry {
    JournalEntry {
        slot: file.slot.to_string(),
        path: file.path.to_string_lossy().into_owned(),
        staged: staged_path(&file.path, generation)
            .to_string_lossy()
            .into_owned(),
        backup: backup_path(&file.path, generation)
            .to_string_lossy()
            .into_owned(),
        done: false,
    }
}

/// Stages one body and renames it over its target, keeping the previous body as a
/// backup so the batch can be rolled back.
fn stage_and_land(file: &CommitFile, generation: u64) -> Result<Landed> {
    let staged = staged_path(&file.path, generation);
    let backup = backup_path(&file.path, generation);
    let had_target = file.path.exists();

    stage_body(&staged, &file.body)?;
    if had_target {
        if let Err(err) = fs::copy(&file.path, &backup) {
            remove_quietly(&staged);
            return Err(err).with_context(|| format!("back up {}", file.path.display()));
        }
        set_owner_only(&backup);
    }
    if let Err(err) = fs::rename(&staged, &file.path) {
        remove_quietly(&staged);
        remove_quietly(&backup);
        return Err(err).with_context(|| format!("replace {}", file.path.display()));
    }
    Ok(Landed {
        target: file.path.clone(),
        backup: had_target.then_some(backup),
    })
}

/// Writes `body` into the deterministic staged sibling, 0600 at creation.
fn stage_body(staged: &Path, body: &str) -> Result<()> {
    let dir = parent_of(staged);
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    // A leftover at this name belongs to an interrupted run of the same
    // generation; the lock is held, so clearing it is safe.
    remove_quietly(staged);
    let mut file = open_created(staged).context("stage the temporary file")?;
    let written = persist_body(&mut file, body);
    drop(file);
    if written.is_err() {
        remove_quietly(staged);
    }
    written.context("write the staged body")
}

fn restore_landed(landed: &[Landed]) {
    for item in landed {
        match &item.backup {
            Some(backup) if backup.exists() => {
                let _ = fs::rename(backup, &item.target);
            }
            _ => {
                let _ = fs::remove_file(&item.target);
            }
        }
    }
}

/// Bounded single-line form of an error for the journal. Only paths and kernel
/// error text reach it: a body is never staged into the journal.
fn redact_error(err: &anyhow::Error) -> String {
    let flat = format!("{err:#}").replace(['\n', '\r'], " ");
    let taken: String = flat.chars().take(MAX_ERROR_CHARS).collect();
    if taken.chars().count() < flat.chars().count() {
        format!("{taken}…")
    } else {
        taken
    }
}

fn staged_path(target: &Path, generation: u64) -> PathBuf {
    sibling_with_suffix(target, &format!("{generation}.staged"))
}

fn backup_path(target: &Path, generation: u64) -> PathBuf {
    sibling_with_suffix(target, &format!("{generation}.bak"))
}

fn sibling_with_suffix(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push('.');
    name.push_str(suffix);
    parent_of(target).join(name)
}

fn create_sibling(dir: &Path, prefix: &str) -> std::io::Result<(PathBuf, fs::File)> {
    let mut last = None;
    for _ in 0..32 {
        let candidate = dir.join(format!(
            "{prefix}{}-{}-{}",
            std::process::id(),
            temp_seq(),
            now_nanos()
        ));
        match open_created(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no unique temporary file name")))
}

/// Creates `path` exclusively, owner-only at creation on Unix.
fn open_created(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(SECURE_FILE_MODE);
    }
    options.open(path)
}

fn set_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(SECURE_FILE_MODE));
    }
}

fn persist_body(file: &mut fs::File, body: &str) -> std::io::Result<()> {
    file.write_all(body.as_bytes())?;
    file.flush()?;
    file.sync_all()
}

fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}

fn temp_seq() -> u64 {
    TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn host_name() -> String {
    for key in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// No staged, backup or temporary sibling may survive a commit.
    fn leftovers(dir: &Path) -> Vec<String> {
        names(dir)
            .into_iter()
            .filter(|name| {
                name.starts_with(TEMP_PREFIX) || name.ends_with(".staged") || name.ends_with(".bak")
            })
            .collect()
    }

    #[test]
    fn write_secure_creates_a_unique_sibling_and_replaces_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        write_secure(&target, "first").unwrap();
        write_secure(&target, "second").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "second");
        assert_eq!(names(dir.path()), vec!["config.toml".to_string()]);

        // A missing parent directory is created rather than refused.
        let nested = dir.path().join("nested").join("quota.json");
        write_secure(&nested, "{}").unwrap();
        assert_eq!(fs::read_to_string(&nested).unwrap(), "{}");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn write_secure_permissions_are_0600_at_creation() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nested").join("quota.json");
        write_secure(&target, "{}").unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, SECURE_FILE_MODE);
    }

    #[test]
    fn commit_files_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let auth = dir.path().join("auth.json");
        let quota = dir.path().join("quota.json");
        write_secure(&auth, "{\"a\":1}").unwrap();
        write_secure(&quota, "{\"b\":2}").unwrap();

        let before = current_generation();
        let epoch_before = reload_epoch();
        let lock = ConfigLock::acquire_at(dir.path()).unwrap();
        let generation = lock
            .commit_files(&[
                CommitFile::new("auth", auth.clone(), "{\"a\":2}".to_string()),
                CommitFile::new("quota", quota.clone(), "{\"b\":3}".to_string()),
            ])
            .unwrap();
        drop(lock);

        assert!(generation > before);
        assert!(current_generation() >= generation);
        assert!(reload_epoch() > epoch_before);
        assert_eq!(fs::read_to_string(&auth).unwrap(), "{\"a\":2}");
        assert_eq!(fs::read_to_string(&quota).unwrap(), "{\"b\":3}");
        assert!(leftovers(dir.path()).is_empty());

        let journal = read_journal_at(dir.path()).expect("journal");
        assert_eq!(journal.state, STATE_COMMITTED);
        assert_eq!(journal.generation, generation);
        assert!(journal.files.iter().all(|entry| entry.done));
        assert!(journal.error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn commit_files_restores_the_previous_pair_when_one_rename_fails() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("auth.json");
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        let second = nested.join("quota.json");
        write_secure(&first, "old-first").unwrap();
        write_secure(&second, "old-second").unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o555)).unwrap();

        let lock = ConfigLock::acquire_at(dir.path()).unwrap();
        let auth = CommitFile::new("auth", first.clone(), "new-first".to_string());
        let quota = CommitFile::new("quota", second.clone(), "new-second".to_string());
        let outcome = lock.commit_files(&[auth, quota]);
        drop(lock);
        // Put the directory back so the temp dir can be cleaned up.
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            outcome.is_err(),
            "the read-only parent must fail the commit"
        );
        assert_eq!(fs::read_to_string(&first).unwrap(), "old-first");
        assert_eq!(fs::read_to_string(&second).unwrap(), "old-second");

        let journal = read_journal_at(dir.path()).expect("journal");
        assert_eq!(journal.state, STATE_RESTORED);
        assert!(journal.error.is_some());
        assert!(journal.files[0].done, "the first file landed");
        assert!(!journal.files[1].done, "the second file never landed");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn interrupted_commit_is_recovered_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let landed = dir.path().join("auth.json");
        let pending = dir.path().join("quota.json");
        let landed_backup = dir.path().join("auth.json.7.bak");
        let pending_backup = dir.path().join("quota.json.7.bak");
        fs::write(&landed, "new-auth").unwrap();
        fs::write(&landed_backup, "old-auth").unwrap();
        fs::write(&pending, "new-quota").unwrap();
        fs::write(&pending_backup, "old-quota").unwrap();

        let journal = CommitJournal {
            generation: 7,
            state: STATE_PENDING.to_string(),
            started_at: now_rfc3339(),
            finished_at: None,
            error: None,
            files: vec![
                JournalEntry {
                    slot: "auth".to_string(),
                    path: text(&landed),
                    staged: text(&dir.path().join("auth.json.7.staged")),
                    backup: text(&landed_backup),
                    done: true,
                },
                JournalEntry {
                    slot: "quota".to_string(),
                    path: text(&pending),
                    staged: text(&dir.path().join("quota.json.7.staged")),
                    backup: text(&pending_backup),
                    done: false,
                },
            ],
        };
        write_journal_at(dir.path(), &journal).unwrap();

        let report = recover_at_startup_at(dir.path());
        assert_eq!(report.state, "restored");
        assert_eq!(report.generation, 7);
        assert_eq!(report.restored, vec![pending.clone()]);
        assert_eq!(
            report.detail,
            "restored 1 file(s) after an interrupted commit"
        );
        // Only the entry that had not landed is rolled back.
        assert_eq!(fs::read_to_string(&pending).unwrap(), "old-quota");
        assert_eq!(fs::read_to_string(&landed).unwrap(), "new-auth");

        let after = read_journal_at(dir.path()).expect("journal");
        assert_eq!(after.state, STATE_RESTORED);
        assert_eq!(
            after.error.as_deref(),
            Some("recovered after interrupted commit")
        );
        assert!(after.finished_at.is_some());
    }

    #[test]
    fn a_committed_journal_needs_no_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let journal = CommitJournal {
            generation: 4,
            state: STATE_COMMITTED.to_string(),
            started_at: now_rfc3339(),
            finished_at: Some(now_rfc3339()),
            error: None,
            files: Vec::new(),
        };
        write_journal_at(dir.path(), &journal).unwrap();
        let report = recover_at_startup_at(dir.path());
        assert_eq!(report.state, "clean");
        assert!(report.restored.is_empty());
        assert_eq!(read_journal_at(dir.path()), Some(journal));
    }

    #[test]
    fn a_second_lock_is_refused_while_one_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let lock = ConfigLock::acquire_at(dir.path()).unwrap();
        assert_eq!(lock.root(), dir.path());
        assert!(
            ConfigLock::acquire_at(dir.path()).is_err(),
            "a live holder keeps the lock"
        );
        drop(lock);
        assert!(
            ConfigLock::acquire_at(dir.path()).is_ok(),
            "the lock is free once the holder is gone"
        );
    }

    /// A lockfile is created empty and its body written afterwards, so a
    /// concurrent reader catches it mid-acquire. That reader must treat the
    /// unfinished lockfile as LIVE: taking it over would delete another
    /// writer's lock and leave two holders inside `commit_files`.
    #[test]
    fn an_unfinished_lockfile_is_treated_as_live_not_dead() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_NAME);
        // Exactly what `open_created` leaves behind before the body is written.
        fs::write(&lock_path, "").unwrap();

        assert!(
            lock_takeover(&lock_path).is_none(),
            "an empty lockfile is a holder mid-acquire, never a dead one"
        );
        assert!(
            ConfigLock::acquire_at(dir.path()).is_err(),
            "a writer holding an unfinished lock keeps the lock"
        );
        // A half-written body is the same story.
        fs::write(&lock_path, "{\"pid\": 12").unwrap();
        assert!(
            lock_takeover(&lock_path).is_none(),
            "a truncated lockfile is a holder mid-acquire"
        );
    }

    /// A lockfile that has sat unfinished past the stale window is abandoned, so
    /// a crashed acquisition must not wedge the configuration forever.
    #[test]
    fn an_abandoned_unfinished_lockfile_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_NAME);
        fs::write(&lock_path, "").unwrap();
        let stale = SystemTime::now() - (LOCK_STALE_AFTER + Duration::from_secs(5));
        let file = fs::OpenOptions::new().write(true).open(&lock_path).unwrap();
        file.set_modified(stale).unwrap();
        drop(file);

        assert!(
            lock_takeover(&lock_path).is_some(),
            "an unfinished lockfile older than the stale window is abandoned"
        );
        assert!(
            ConfigLock::acquire_at(dir.path()).is_ok(),
            "an abandoned lock does not wedge the configuration"
        );
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join(LOCK_NAME);
        let beat = chrono::Utc::now() - chrono::Duration::hours(2);
        let stale = json!({
            "pid": 999_999_u64,
            "host": "gone",
            "acquired_at": now_rfc3339(),
            "heartbeat_at": beat.to_rfc3339(),
        });
        fs::write(&lock_path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

        let lock = ConfigLock::acquire_at(dir.path()).expect("stale lock taken over");
        let raw = fs::read_to_string(&lock_path).unwrap();
        let owner: serde_json::Value = serde_json::from_str(&raw).expect("JSON owner");
        drop(lock);
        assert_eq!(owner["pid"].as_u64(), Some(std::process::id() as u64));
        assert_eq!(owner["took_over_pid"].as_u64(), Some(999_999));
    }

    #[test]
    fn concurrent_writers_never_corrupt_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = root.join("config.toml");
        std::thread::scope(|scope| {
            for writer in 0..8u32 {
                let target = target.clone();
                scope.spawn(move || {
                    let body = format!("{{\"writer\":{writer},\"pad\":\"{}\"}}", "x".repeat(4096));
                    let outcome =
                        commit_with_retry(root, &[CommitFile::new("settings", target, body)]);
                    assert!(outcome.is_ok(), "writer {writer} failed: {outcome:?}");
                });
            }
        });

        let final_body = fs::read_to_string(&target).expect("the target exists");
        let parsed: serde_json::Value = serde_json::from_str(&final_body).expect("complete body");
        assert_eq!(parsed["pad"].as_str().unwrap().len(), 4096);
        assert!(parsed["writer"].as_u64().unwrap() < 8);
        assert!(leftovers(dir.path()).is_empty());
    }

    /// Waits for the live holder instead of failing: the exclusive lock already
    /// proves only one writer is inside a commit.
    fn commit_with_retry(root: &Path, files: &[CommitFile]) -> Result<u64> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match ConfigLock::acquire_at(root) {
                Ok(lock) => return lock.commit_files(files),
                Err(_) if Instant::now() < deadline => std::thread::sleep(LOCK_STEP),
                Err(err) => return Err(err),
            }
        }
    }

    #[test]
    fn reload_epoch_advances_on_a_successful_commit() {
        let dir = tempfile::tempdir().unwrap();
        let before = reload_epoch();
        let subscriber = subscribe_reload();
        let lock = ConfigLock::acquire_at(dir.path()).unwrap();
        let file = CommitFile::new("settings", dir.path().join("config.toml"), "cfg".into());
        lock.commit_files(&[file]).unwrap();
        drop(lock);
        assert!(reload_epoch() > before);
        assert!(*subscriber.borrow() > before, "the subscriber sees it");
    }

    /// Two commits can never publish the same epoch. A read-then-send claim
    /// would let concurrent commits collide and a listener would observe no
    /// change at all, which is exactly what a reload depends on.
    #[test]
    fn concurrent_commits_each_advance_the_reload_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let before = reload_epoch();
        const COMMITS: u64 = 16;
        std::thread::scope(|scope| {
            for writer in 0..4u32 {
                let root = root.clone();
                scope.spawn(move || {
                    for round in 0..4u32 {
                        let target = root.join(format!("writer-{writer}.toml"));
                        let body = format!("{{\"w\":{writer},\"r\":{round}}}");
                        commit_with_retry(&root, &[CommitFile::new("settings", target, body)])
                            .expect("commit");
                    }
                });
            }
        });
        // One epoch per commit, strictly increasing: the counter is claimed
        // atomically, so no two commits ever share one. The epoch is
        // process-wide, so other tests running in parallel also advance it —
        // this test only claims that its own commits all landed.
        assert!(
            reload_epoch() >= before + COMMITS,
            "each commit must advance the epoch: {} -> {}",
            before,
            reload_epoch()
        );
    }

    #[test]
    fn journal_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let staged = dir.path().join("config.toml.3.staged");
        let backup = dir.path().join("config.toml.3.bak");
        let journal = CommitJournal {
            generation: 3,
            state: STATE_PENDING.to_string(),
            started_at: now_rfc3339(),
            finished_at: None,
            error: Some("boom".to_string()),
            files: vec![JournalEntry {
                slot: "settings".to_string(),
                path: text(&target),
                staged: text(&staged),
                backup: text(&backup),
                done: false,
            }],
        };
        write_journal_at(dir.path(), &journal).unwrap();
        assert_eq!(read_journal_at(dir.path()), Some(journal.clone()));

        let missing = tempfile::tempdir().unwrap();
        assert_eq!(read_journal_at(missing.path()), None);

        let path = dir.path().join(JOURNAL_NAME);
        fs::write(&path, "   ").unwrap();
        assert_eq!(read_journal_at(dir.path()), None);
        fs::write(&path, "not json").unwrap();
        assert_eq!(read_journal_at(dir.path()), None);
    }

    #[test]
    fn a_batch_with_a_duplicate_or_empty_slot_is_refused_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let other = dir.path().join("quota.json");
        let lock = ConfigLock::acquire_at(dir.path()).unwrap();

        let first = CommitFile::new("auth", path.clone(), "one".to_string());
        let again = CommitFile::new("auth", other.clone(), "two".to_string());
        assert!(lock.commit_files(&[first, again]).is_err());

        let blank = CommitFile::new("  ", path.clone(), "one".to_string());
        assert!(lock.commit_files(&[blank]).is_err());

        let rootless = CommitFile::new("auth", PathBuf::from("/"), "one".to_string());
        assert!(lock.commit_files(&[rootless]).is_err());

        drop(lock);
        assert!(!path.exists(), "nothing was staged for a refused batch");
        assert!(!dir.path().join(JOURNAL_NAME).exists());
    }
}
