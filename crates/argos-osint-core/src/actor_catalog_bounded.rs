//! Incrementally bounded actor CSV parsing before any canonical source write.
//! Collection and streaming share the same row validation and resource policy.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use anyhow::bail;
use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;

use super::curated_aliases;
use super::Actor;
use crate::brain_actors;

/// Independent input, record, count and retained-object limits. This is a
/// trusted local parser policy, not client-deserializable execution authority.
#[derive(Clone, Debug)]
pub struct ActorReadLimits {
    pub max_file_bytes: u64,
    pub max_record_bytes: usize,
    pub max_rows: usize,
    pub max_retained_bytes: u64,
}

impl Default for ActorReadLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024 * 1024,
            max_record_bytes: 64 * 1024,
            max_rows: 100_000,
            max_retained_bytes: 32 * 1024 * 1024,
        }
    }
}

impl ActorReadLimits {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=256 * 1024 * 1024).contains(&self.max_file_bytes)
                && (1024..=64 * 1024).contains(&self.max_record_bytes)
                && (1..=100_000).contains(&self.max_rows)
                && (1024 * 1024..=32 * 1024 * 1024).contains(&self.max_retained_bytes),
            "actor_parse_invalid_limits"
        );
        Ok(())
    }
}

fn actor_bytes(actor: &Actor) -> Result<u64> {
    // Charge actual capacities plus conservative BTree node/allocator overhead.
    // Vec<Actor> slots are charged at reserve time, including unused capacity.
    let mut bytes = 128_u64; // source-ID uniqueness tree node.
    let mut add = |value: usize| -> Result<()> {
        bytes = bytes
            .checked_add(value as u64)
            .context("actor_parse_memory_budget")?;
        Ok(())
    };
    for value in [&actor.id, &actor.source_id, &actor.name, &actor.subtype] {
        add(value.capacity())?;
    }
    add(actor.source_id.len())?; // retained uniqueness key clone.
    add(actor.aliases.capacity() * std::mem::size_of::<String>())?;
    for value in &actor.aliases {
        add(value.capacity())?;
    }
    for (key, value) in &actor.metadata {
        add(128)?;
        add(key.capacity())?;
        add(value.capacity())?;
    }
    Ok(bytes)
}

#[derive(Clone, Copy)]
enum QuoteState {
    Start,
    Plain,
    Quoted,
    Closed,
}

/// Bound raw RFC records before csv/StringRecord/JSON can allocate for them.
struct BoundedInput<'policy, R> {
    inner: R,
    limits: &'policy ActorReadLimits,
    cancel: &'policy AtomicBool,
    total: u64,
    record: usize,
    previous_record: usize,
    state: QuoteState,
    after_cr: bool,
    prefix: Vec<u8>,
    rejected: bool,
}

impl<'policy, R: Read> BoundedInput<'policy, R> {
    fn new(inner: R, limits: &'policy ActorReadLimits, cancel: &'policy AtomicBool) -> Self {
        Self {
            inner,
            limits,
            cancel,
            total: 0,
            record: 0,
            previous_record: 0,
            state: QuoteState::Start,
            after_cr: false,
            prefix: Vec::with_capacity(3),
            rejected: false,
        }
    }
    fn inspect(&mut self, byte: u8) -> io::Result<()> {
        self.total += 1;
        self.record += 1;
        if self.total > self.limits.max_file_bytes || self.record > self.limits.max_record_bytes {
            return Err(io::Error::other("actor_parse_raw_record_budget"));
        }
        if self.total <= 3 {
            self.prefix.push(byte);
            if matches!(
                self.prefix.as_slice(),
                b"\xef" | b"\xef\xbb" | b"\xef\xbb\xbf"
            ) {
                return Ok(());
            }
        }
        if self.after_cr {
            self.after_cr = false;
            if byte == b'\n' {
                if self.previous_record + 1 > self.limits.max_record_bytes {
                    return Err(io::Error::other("actor_parse_raw_record_budget"));
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
                _ => return Err(io::Error::other("actor_parse_malformed_csv")),
            },
            QuoteState::Start => match byte {
                b'"' => self.state = QuoteState::Quoted,
                b',' => (),
                b'\r' | b'\n' => self.end_record(byte),
                _ => self.state = QuoteState::Plain,
            },
            QuoteState::Plain => match byte {
                b'"' => return Err(io::Error::other("actor_parse_malformed_csv")),
                b',' => self.state = QuoteState::Start,
                b'\r' | b'\n' => self.end_record(byte),
                _ => (),
            },
        }
        Ok(())
    }
    fn end_record(&mut self, byte: u8) {
        self.previous_record = self.record;
        self.record = 0;
        self.state = QuoteState::Start;
        self.after_cr = byte == b'\r';
    }
}

impl<R: Read> Read for BoundedInput<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.rejected || self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::other("actor_parse_cancelled_or_rejected"));
        }
        let length = self.inner.read(output)?;
        for byte in &output[..length] {
            if let Err(error) = self.inspect(*byte) {
                self.rejected = true;
                return Err(error);
            }
        }
        if length == 0 && matches!(self.state, QuoteState::Quoted) {
            self.rejected = true;
            return Err(io::Error::other("actor_parse_unterminated_quote"));
        }
        Ok(length)
    }
}

/// Full-scan observations without retaining the decoded catalog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct ActorScan {
    pub rows: usize,
    pub peak_retained_bytes: u64,
}

/// Validate every actor while retaining only unique IDs and one decoded record.
///
/// Visitors must independently bound any output they retain. An error or
/// cancellation stops scanning immediately and is never converted to success.
pub fn scan_bounded(
    reader: impl Read,
    limits: &ActorReadLimits,
    cancel: &AtomicBool,
    mut resource_probe: impl FnMut(u64) -> Result<()>,
    mut visit: impl FnMut(Actor) -> Result<()>,
) -> Result<ActorScan> {
    limits.validate()?;
    resource_probe(0)?;
    let mut rows = 0_usize;
    let summary = decode_bounded(reader, limits, cancel, |actor, retained| {
        rows += 1;
        if rows.is_multiple_of(128) {
            resource_probe(retained)?;
        }
        visit(actor)
    })?;
    resource_probe(summary.peak_retained_bytes)?;
    Ok(summary)
}

/// Collect validated actors under the existing aggregate retained-Vec budget.
pub fn read_bounded(
    reader: impl Read,
    limits: &ActorReadLimits,
    cancel: &AtomicBool,
    mut resource_probe: impl FnMut(u64) -> Result<()>,
) -> Result<Vec<Actor>> {
    limits.validate()?;
    resource_probe(0)?;
    let mut actors = Vec::new();
    let mut retained_bytes = 0_u64;
    decode_bounded(reader, limits, cancel, |actor, _| {
        retained_bytes = retained_bytes
            .checked_add(actor_bytes(&actor)?)
            .context("actor_parse_memory_budget")?;
        ensure!(
            retained_bytes <= limits.max_retained_bytes,
            "actor_parse_memory_budget"
        );
        if actors.len() == actors.capacity() {
            let requested = (limits.max_rows - actors.len()).min(128);
            let before = actors.capacity();
            let reserve = (requested as u64)
                .checked_mul(std::mem::size_of::<Actor>() as u64)
                .context("actor_parse_memory_budget")?;
            ensure!(
                retained_bytes
                    .checked_add(reserve)
                    .is_some_and(|next| next <= limits.max_retained_bytes),
                "actor_parse_memory_budget"
            );
            actors
                .try_reserve_exact(requested)
                .context("actor_parse_allocation_failed")?;
            let allocated =
                (actors.capacity() - before) as u64 * std::mem::size_of::<Actor>() as u64;
            retained_bytes = retained_bytes
                .checked_add(allocated)
                .context("actor_parse_memory_budget")?;
            ensure!(
                retained_bytes <= limits.max_retained_bytes,
                "actor_parse_memory_budget"
            );
        }
        actors.push(actor);
        if actors.len() % 128 == 0 {
            resource_probe(retained_bytes)?;
        }
        Ok(())
    })?;
    resource_probe(retained_bytes)?;
    Ok(actors)
}

fn decode_bounded(
    reader: impl Read,
    limits: &ActorReadLimits,
    cancel: &AtomicBool,
    mut visit: impl FnMut(Actor, u64) -> Result<()>,
) -> Result<ActorScan> {
    let mut csv = csv::Reader::from_reader(BoundedInput::new(reader, limits, cancel));
    let headers = csv.headers()?.clone();
    ensure!(
        headers.len() <= 64 && headers.iter().all(|value| value.len() <= 256),
        "actor_csv_header_bound"
    );
    let unique: BTreeSet<_> = headers.iter().collect();
    ensure!(unique.len() == headers.len(), "actor_csv_duplicate_header");
    for required in [
        "actor_id",
        "actor_name",
        "actor_type",
        "aliases",
        "name_normalized",
    ] {
        if !headers.iter().any(|h| h == required) {
            bail!("actor CSV is missing {required}");
        }
    }
    let mut ids = BTreeSet::new();
    let mut id_bytes = 0_u64;
    let mut summary = ActorScan::default();
    for (index, record) in csv.records().enumerate() {
        ensure!(!cancel.load(Ordering::Acquire), "actor_parse_cancelled");
        ensure!(index < limits.max_rows, "actor_parse_row_budget");
        let record = record.with_context(|| format!("actor CSV record {}", index + 1))?;
        let metadata: BTreeMap<String, String> = headers
            .iter()
            .zip(record.iter())
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let source_id = metadata["actor_id"].trim().to_owned();
        let name = metadata["actor_name"].trim().to_owned();
        if source_id.is_empty() || name.is_empty() || ids.contains(&source_id) {
            bail!(
                "actor CSV record {} has an empty name/ID or duplicate ID",
                index + 1
            );
        }
        let mut aliases: Vec<String> = serde_json::from_str(&metadata["aliases"])
            .with_context(|| format!("invalid aliases for {source_id}"))?;
        aliases.extend([name.clone(), metadata["name_normalized"].clone()]);
        aliases.extend(curated_aliases(&source_id).iter().map(|s| (*s).to_owned()));
        aliases.retain(|a| !brain_actors::normalize_alias(a).is_empty());
        aliases.sort();
        aliases.dedup();
        let normalized: BTreeSet<_> = aliases
            .iter()
            .map(|alias| brain_actors::normalize_alias(alias))
            .collect();
        let alias_bytes = aliases
            .iter()
            .chain(normalized.iter())
            .try_fold(0_usize, |bytes, value| bytes.checked_add(value.len()))
            .context("actor_parse_identity_expansion_budget")?;
        // Preserve the real catalog's richer alias sets. Bound normalized
        // writer plans and aggregate text rather than truncating source data.
        ensure!(
            source_id.len() <= 256
                && name.len() <= 1024
                && normalized.len() <= 512
                && aliases.iter().all(|value| value.len() <= 1024)
                && normalized.iter().all(|value| value.len() <= 1024)
                && alias_bytes <= 128 * 1024,
            "actor_parse_identity_expansion_budget"
        );
        let normalization_bytes = normalized
            .iter()
            .try_fold(0_u64, |bytes, value| {
                bytes.checked_add(128 + value.capacity() as u64)
            })
            .context("actor_parse_memory_budget")?;
        drop(normalized);
        let subtype = match metadata["actor_type"].as_str() {
            "state" | "territory" | "country" => "place",
            "person" | "individual" => "person",
            "organization"
            | "organisation"
            | "company"
            | "legal_entity"
            | "group"
            | "government"
            | "cyber_intrusion_set"
            | "identity_group"
            | "informal_supporter_group"
            | "organized_armed_group" => "organization",
            _ => "unknown",
        }
        .to_owned();
        let actor = Actor {
            id: brain_actors::hid("catalog-actor", &[&source_id]),
            source_id,
            name,
            subtype,
            aliases,
            metadata,
        };
        let current = id_bytes
            .checked_add(actor_bytes(&actor)?)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Actor>() as u64))
            .and_then(|bytes| bytes.checked_add(normalization_bytes))
            .context("actor_parse_memory_budget")?;
        ensure!(
            current <= limits.max_retained_bytes,
            "actor_parse_memory_budget"
        );
        id_bytes = id_bytes
            .checked_add(128 + actor.source_id.len() as u64)
            .context("actor_parse_memory_budget")?;
        ids.insert(actor.source_id.clone());
        summary.rows += 1;
        summary.peak_retained_bytes = summary.peak_retained_bytes.max(current);
        visit(actor, current)?;
    }
    ensure!(summary.rows > 0, "actor CSV contains no records");
    ensure!(!cancel.load(Ordering::Acquire), "actor_parse_cancelled");
    Ok(summary)
}

/// Conservative transaction payload estimate for an already validated actor.
///
/// The seed writer keeps one provenance allocation and lightweight alias plans,
/// materializing alias INSERT packets under a separate bounded chunk policy.
/// Include those plans and binding copies before selecting a row batch.
pub fn actor_seed_payload_bytes(actor: &Actor) -> Result<u64> {
    let provenance = serde_json::to_vec(&actor.metadata)?.len() as u64;
    let catalog = serde_json::to_vec(actor)?.len() as u64;
    let mut keys = BTreeSet::new();
    let mut alias_bytes = 0_u64;
    for alias in std::iter::once(&actor.name).chain(&actor.aliases) {
        let normalized = brain_actors::normalize_alias(alias);
        if normalized.chars().count() < 3 || !keys.insert(normalized.clone()) {
            continue;
        }
        let text = (actor.id.len() as u64)
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(alias.len() as u64))
            .and_then(|bytes| bytes.checked_add(normalized.len() as u64))
            .and_then(|bytes| bytes.checked_add(512))
            .context("actor_seed_payload_budget")?;
        alias_bytes = alias_bytes
            .checked_add(text)
            .context("actor_seed_payload_budget")?;
    }
    catalog
        .checked_mul(3)
        .and_then(|catalog| provenance.checked_add(catalog))
        .and_then(|bytes| bytes.checked_add(alias_bytes))
        .and_then(|bytes| bytes.checked_add(2048))
        .and_then(|bytes| bytes.checked_mul(4))
        .context("actor_seed_payload_budget")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV:&[u8]=b"actor_id,actor_name,actor_type,aliases,name_normalized,description\r\nsynthetic-1,Harbor Union,organization,\"[\"\"HU Catalog\"\"]\",harbor union,\"multi\nline\"\r\n";

    #[test]
    fn bounded_parser_preserves_existing_csv_semantics_and_checks_resources_before_return() {
        let cancel = AtomicBool::new(false);
        let mut probes = 0;
        let actual = read_bounded(CSV, &ActorReadLimits::default(), &cancel, |_| {
            probes += 1;
            Ok(())
        })
        .expect("bounded quoted CSV");
        assert_eq!(
            actual,
            super::super::read(CSV).expect("existing semantic parser")
        );
        assert!(probes >= 2);
        assert!(
            read_bounded(CSV, &ActorReadLimits::default(), &cancel, |retained| {
                ensure!(retained == 0, "post-parse pressure");
                Ok(())
            })
            .is_err()
        );
    }

    #[test]
    fn record_row_retained_memory_and_shutdown_are_bounded_before_source_publication() {
        let cancel = AtomicBool::new(false);
        let header = b"actor_id,actor_name,actor_type,aliases,name_normalized\n";
        let mut oversized = header.to_vec();
        oversized.extend_from_slice(b"id,\"");
        oversized.extend(vec![b'x'; 64 * 1024]);
        assert!(read_bounded(
            oversized.as_slice(),
            &ActorReadLimits::default(),
            &cancel,
            |_| Ok(())
        )
        .is_err());
        let mut rows = header.to_vec();
        rows.extend_from_slice(b"one,Harbor One,organization,[],harbor one\ntwo,Harbor Two,organization,[],harbor two\n");
        let limits = ActorReadLimits {
            max_rows: 1,
            ..Default::default()
        };
        assert!(read_bounded(rows.as_slice(), &limits, &cancel, |_| Ok(())).is_err());
        let limits = ActorReadLimits {
            max_retained_bytes: 1024 * 1024,
            ..Default::default()
        };
        let mut many = header.to_vec();
        for index in 0..1500 {
            many.extend_from_slice(
                format!("id-{index},Harbor {index},organization,[],harbor {index}\n").as_bytes(),
            );
        }
        assert!(read_bounded(many.as_slice(), &limits, &cancel, |_| Ok(())).is_err());
        cancel.store(true, Ordering::Release);
        assert!(read_bounded(CSV, &ActorReadLimits::default(), &cancel, |_| Ok(())).is_err());
    }

    #[test]
    fn streaming_scan_preserves_rows_without_collecting_their_metadata() {
        let description = "x".repeat(2048);
        let mut csv =
            String::from("actor_id,actor_name,actor_type,aliases,name_normalized,description\n");
        for index in 0..1000 {
            csv.push_str(&format!(
                "id-{index},Harbor {index},organization,[],harbor {index},{description}\n"
            ));
        }
        let limits = ActorReadLimits {
            max_retained_bytes: 1024 * 1024,
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        assert!(read_bounded(csv.as_bytes(), &limits, &cancel, |_| Ok(())).is_err());
        let mut visited = 0;
        let summary = scan_bounded(
            csv.as_bytes(),
            &limits,
            &cancel,
            |_| Ok(()),
            |actor| {
                assert_eq!(actor.source_id, format!("id-{visited}"));
                assert_eq!(actor.metadata["description"], description);
                visited += 1;
                Ok(())
            },
        )
        .expect("whole CSV fits the streaming ID/one-record budget");
        assert_eq!(visited, 1000);
        assert_eq!(summary.rows, visited);
        assert!(summary.peak_retained_bytes < limits.max_retained_bytes);
    }

    #[test]
    fn scanner_propagates_visitor_failure_and_cancellation_before_more_visits() {
        let limits = ActorReadLimits::default();
        let cancel = AtomicBool::new(false);
        let failed = scan_bounded(
            CSV,
            &limits,
            &cancel,
            |_| Ok(()),
            |_| bail!("synthetic_visitor_failure"),
        );
        assert_eq!(
            failed
                .expect_err("visitor failure is not swallowed")
                .to_string(),
            "synthetic_visitor_failure"
        );
        let mut visited = 0;
        let cancelled = scan_bounded(
            CSV,
            &limits,
            &cancel,
            |_| Ok(()),
            |_| {
                visited += 1;
                cancel.store(true, Ordering::Release);
                Ok(())
            },
        );
        assert!(cancelled.is_err());
        assert_eq!(visited, 1);
    }

    #[test]
    fn scanner_keeps_identity_and_record_validation_and_accounts_alias_expansion() {
        let cancel = AtomicBool::new(false);
        let mut duplicate = b"actor_id,actor_name,actor_type,aliases,name_normalized\n".to_vec();
        duplicate.extend_from_slice(
            b"same,Harbor One,organization,[],harbor one\nsame,Harbor Two,organization,[],harbor two\n",
        );
        assert!(scan_bounded(
            duplicate.as_slice(),
            &ActorReadLimits::default(),
            &cancel,
            |_| Ok(()),
            |_| Ok(())
        )
        .is_err());
        let actor = read_bounded(CSV, &ActorReadLimits::default(), &cancel, |_| Ok(()))
            .expect("validated fixture")
            .pop()
            .expect("one fixture actor");
        let baseline = actor_seed_payload_bytes(&actor).expect("payload estimate");
        let mut expanded = actor.clone();
        expanded.aliases.push("A separate normalized alias".into());
        assert!(actor_seed_payload_bytes(&expanded).expect("expanded payload") > baseline);
        let mut repeated = actor.clone();
        repeated.aliases.push(actor.name.clone());
        // Duplicate aliases do not create another provenance-bearing writer row.
        // Their catalog bytes remain charged, so the total may grow slightly.
        let repeated = actor_seed_payload_bytes(&repeated).expect("duplicate payload");
        assert!(repeated >= baseline);
        assert!(repeated < actor_seed_payload_bytes(&expanded).unwrap());
    }

    #[test]
    fn richer_alias_sets_are_preserved_under_normalized_count_and_text_bounds() {
        let encode = |aliases: &[String]| {
            let mut csv = csv::Writer::from_writer(Vec::new());
            csv.write_record([
                "actor_id",
                "actor_name",
                "actor_type",
                "aliases",
                "name_normalized",
                "description",
            ])
            .unwrap();
            csv.write_record([
                "synthetic-rich",
                &"n".repeat(380),
                "organization",
                &serde_json::to_string(aliases).unwrap(),
                "normalized rich actor",
                "all source metadata survives",
            ])
            .unwrap();
            csv.into_inner().unwrap()
        };
        let mut aliases: Vec<_> = (0..258)
            .map(|index| format!("distinct catalog alias {index}"))
            .collect();
        aliases.push("x".repeat(700));
        let csv = encode(&aliases);
        let cancel = AtomicBool::new(false);
        let actors = read_bounded(csv.as_slice(), &ActorReadLimits::default(), &cancel, |_| {
            Ok(())
        })
        .expect("real catalog-sized alias sets fit the bounded policy");
        assert_eq!(actors.len(), 1);
        let actor = &actors[0];
        assert_eq!(actor.name.len(), 380);
        for alias in &aliases {
            assert!(actor.aliases.contains(alias));
        }
        assert_eq!(
            actor.metadata["description"],
            "all source metadata survives"
        );
        assert!(actor_seed_payload_bytes(actor).unwrap() < 2 * 1024 * 1024);

        for aliases in [
            (0..513)
                .map(|index| format!("distinct catalog alias {index}"))
                .collect::<Vec<_>>(),
            vec!["x".repeat(1025)],
        ] {
            let csv = encode(&aliases);
            let error = scan_bounded(
                csv.as_slice(),
                &ActorReadLimits::default(),
                &cancel,
                |_| Ok(()),
                |_| Ok(()),
            )
            .expect_err("identity expansion remains bounded");
            assert_eq!(error.to_string(), "actor_parse_identity_expansion_budget");
        }
    }
}
