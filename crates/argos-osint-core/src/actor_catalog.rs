//! Deterministic, provenance-preserving actor catalog. @tag:brain-memory
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use aho_corasick::AhoCorasick;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::atlas_metadata::{ArticleActor, ArticleMention};
use crate::brain_actors;
use crate::store::AtlasArticleRow;

#[path = "actor_catalog_bounded.rs"]
mod bounded;
pub use bounded::read_bounded;
pub use bounded::ActorReadLimits;

pub const SCOPE: &str = "dataset:argos_actors";
pub const VERSION: &str = "argos-actors-catalog-v1";

/// One catalog identity, retaining every CSV column as source metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Actor {
    pub id: String,
    pub source_id: String,
    pub name: String,
    pub subtype: String,
    pub aliases: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

/// Validate the entire CSV before any database write. Quoting, embedded newlines
/// and JSON alias arrays follow their actual encodings rather than line splits.
pub fn read(reader: impl Read) -> Result<Vec<Actor>> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv.headers()?.clone();
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
    let mut actors = Vec::new();
    for (index, record) in csv.records().enumerate() {
        let record = record.with_context(|| format!("actor CSV record {}", index + 1))?;
        let metadata: BTreeMap<String, String> = headers
            .iter()
            .zip(record.iter())
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let source_id = metadata["actor_id"].trim().to_owned();
        let name = metadata["actor_name"].trim().to_owned();
        if source_id.is_empty() || name.is_empty() || !ids.insert(source_id.clone()) {
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
        actors.push(Actor {
            id: brain_actors::hid("catalog-actor", &[&source_id]),
            source_id,
            name,
            subtype,
            aliases,
            metadata,
        });
    }
    if actors.is_empty() {
        bail!("actor CSV contains no records");
    }
    Ok(actors)
}

/// Curated short forms tied to catalog source identifiers, never inferred
/// surnames. Keep automatic matching and transactional binding on the same
/// alias set.
pub(crate) const CURATED: &[(&str, &[&str])] = &[("os:Q22686", &["Trump"])];

pub(crate) fn curated_aliases(source_id: &str) -> &'static [&'static str] {
    CURATED
        .iter()
        .find(|(id, _)| *id == source_id)
        .map_or(&[], |(_, aliases)| *aliases)
}

/// The curated source identifiers with a short form normalizing to
/// `normalized` — the reverse lookup the alias search service uses to resolve
/// a curated form even before the alias-table backfill ran.
pub(crate) fn curated_sources_for(normalized: &str) -> Vec<&'static str> {
    CURATED
        .iter()
        .filter(|(_, aliases)| {
            aliases
                .iter()
                .any(|alias| brain_actors::normalize_alias(alias) == normalized)
        })
        .map(|(id, _)| *id)
        .collect()
}

/// Shortest normalized form the matcher may match automatically. One- and
/// two-letter strings ("HU", "AI", "UK") collide across actors and headlines,
/// so they are metadata only: never auto-bound, never a strong alias.
pub const MIN_MATCH_CHARS: usize = 3;

/// Whether a normalized alias/query is long enough to participate in
/// automatic matching. Shared by the matcher and the discovery services so
/// both apply the same risky-short-form guard.
pub fn matchable_alias(normalized: &str) -> bool {
    normalized.chars().count() >= MIN_MATCH_CHARS
}

/// Process-wide matcher-generation epoch. Every mutation that changes the
/// catalog's matcher inputs (a completed reseed, a merge touching a catalog
/// actor) bumps it; each repository compares its served matcher against this
/// epoch and rebuilds single-flight when it moved. In-process only: a reseed
/// in another process keeps its fresh cache per repository (unchanged
/// posture), so this is invalidation signaling, not a distributed contract.
static CATALOG_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The epoch the calling process has observed (see [`bump_catalog_epoch`]).
pub fn catalog_epoch() -> u64 {
    CATALOG_EPOCH.load(std::sync::atomic::Ordering::Acquire)
}

/// Signal that catalog matcher inputs changed; repositories rebuild their
/// cached matcher single-flight before its next use.
pub fn bump_catalog_epoch() {
    CATALOG_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

/// Exact normalized word-boundary matching. Shared aliases stay ambiguous;
/// weak_aliases are metadata only and never participate in automatic binding.
/// The matcher carries the content revision it was built from, so discovery
/// responses and receipts can pin the catalog generation they used.
pub struct Matcher {
    actors: Vec<Actor>,
    patterns: AhoCorasick,
    owners: Vec<Vec<usize>>,
    aliases: BTreeMap<String, Vec<usize>>,
    revision: String,
}

impl Matcher {
    pub fn new(mut actors: Vec<Actor>) -> Result<Self> {
        for actor in &mut actors {
            actor.aliases.extend(
                curated_aliases(&actor.source_id)
                    .iter()
                    .map(|s| (*s).to_owned()),
            );
        }
        let mut names: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut revision = String::new();
        for (index, actor) in actors.iter().enumerate() {
            revision.push_str(&actor.id);
            revision.push('\u{1f}');
            revision.push_str(&actor.name);
            revision.push('\u{1f}');
            for alias in &actor.aliases {
                revision.push_str(alias);
                revision.push('\u{1f}');
            }
            for alias in std::iter::once(&actor.name).chain(&actor.aliases) {
                let key = brain_actors::normalize_alias(alias);
                if !matchable_alias(&key) {
                    continue;
                }
                names.entry(format!(" {key} ")).or_default().insert(index);
            }
        }
        let patterns = AhoCorasick::new(names.keys())?;
        Ok(Self {
            actors,
            patterns,
            aliases: names
                .iter()
                .map(|(key, owners)| (key.trim().to_owned(), owners.iter().copied().collect()))
                .collect(),
            owners: names
                .into_values()
                .map(|o| o.into_iter().collect())
                .collect(),
            revision: crate::evidence::content_hash(&revision),
        })
    }

    /// Content revision of the catalog rows this matcher was built from;
    /// equal across matchers built from the same catalog state.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Canonical article actors with byte offsets into the original title and
    /// description, independent of NLP availability. Ambiguous names are omitted.
    pub fn article_actors(&self, article: &AtlasArticleRow) -> Vec<ArticleActor> {
        self.fields(&[
            ("title", &article.title),
            ("description", &article.description),
        ])
    }

    /// Resolve only NLP person/organization candidates against unique strong
    /// catalog aliases, then add every unambiguous exact alias occurrence in
    /// the body. Exact matching never needs NLP (Phase 4); NLP candidates
    /// enrich the resolved spellings and mention spans. Unknown and ambiguous
    /// names never create identities.
    pub fn body_actors(
        &self,
        text: &str,
        candidates: &[crate::nlp::EntityCandidate],
    ) -> Vec<ArticleActor> {
        // The NLP-gated pass first: candidates anchored to their own spans.
        let mut found = Vec::new();
        for candidate in candidates {
            if !matches!(candidate.type_hint.as_str(), "person" | "organization") {
                continue;
            }
            let key = brain_actors::normalize_alias(&candidate.comparison_key);
            let Some(owners) = self.aliases.get(&key) else {
                continue;
            };
            let [owner] = owners.as_slice() else { continue };
            let actor = &self.actors[*owner];
            let mut resolved = ArticleActor {
                id: actor.id.clone(),
                type_hint: actor.subtype.clone(),
                display_name: actor.name.clone(),
                spellings: Vec::new(),
                mentions: Vec::new(),
            };
            for span in &candidate.mention_refs {
                let Some(original) = span.slice(text) else {
                    continue;
                };
                if brain_actors::normalize_alias(original) != key {
                    continue;
                }
                if !resolved.spellings.iter().any(|s| s == original) {
                    resolved.spellings.push(original.to_owned());
                }
                resolved.mentions.push(ArticleMention {
                    type_hint: actor.subtype.clone(),
                    comparison_key: key.clone(),
                    field: "body".into(),
                    start_byte: span.start_byte,
                    end_byte: span.end_byte,
                    original_text: original.to_owned(),
                });
            }
            if !resolved.mentions.is_empty() {
                append_body_actors(&mut found, vec![resolved]);
            }
        }
        // The NLP-off pass: the same word-boundary matcher that reads
        // titles/descriptions reads the cleaned body. Unambiguous aliases
        // bind; ambiguous and short ones never do.
        append_body_actors(&mut found, self.fields(&[("body", text)]));
        found
    }

    fn fields(&self, fields: &[(&str, &str)]) -> Vec<ArticleActor> {
        let mut found: BTreeMap<usize, ArticleActor> = BTreeMap::new();
        for &(field, text) in fields {
            let (normalized, offsets) = normalized_offsets(text);
            let matches: Vec<_> = self.patterns.find_overlapping_iter(&normalized).collect();
            for matched in &matches {
                let owners = &self.owners[matched.pattern().as_usize()];
                let [owner] = owners.as_slice() else {
                    continue;
                };
                let actor = &self.actors[*owner];
                let alias = normalized[matched.start()..matched.end()].trim();
                let curated = curated_aliases(&actor.source_id)
                    .iter()
                    .any(|name| brain_actors::normalize_alias(name) == alias);
                // A curated short form must not override another person's
                // longer exact name, e.g. Melania Trump versus bare Trump.
                if curated && matches.iter().any(|other| {
                    other.start() <= matched.start() && other.end() >= matched.end()
                        && (other.start() < matched.start() || other.end() > matched.end())
                        && matches!(self.owners[other.pattern().as_usize()].as_slice(), [id] if id != owner)
                }) { continue; }
                let start = offsets[matched.start() + 1].0;
                let end = offsets[matched.end() - 2].1;
                let original_text = text[start..end].to_owned();
                let out = found.entry(*owner).or_insert_with(|| ArticleActor {
                    id: actor.id.clone(),
                    type_hint: actor.subtype.clone(),
                    display_name: actor.name.clone(),
                    spellings: Vec::new(),
                    mentions: Vec::new(),
                });
                if !out.spellings.contains(&original_text) {
                    out.spellings.push(original_text.clone());
                }
                let mention = ArticleMention {
                    type_hint: actor.subtype.clone(),
                    comparison_key: brain_actors::normalize_alias(&original_text),
                    field: field.to_owned(),
                    start_byte: start,
                    end_byte: end,
                    original_text,
                };
                if !out.mentions.contains(&mention) {
                    out.mentions.push(mention);
                }
            }
        }
        found.into_values().collect()
    }
}
/// Append identities discovered in a clean body without moving metadata actors.
pub(crate) fn append_body_actors(actors: &mut Vec<ArticleActor>, body_actors: Vec<ArticleActor>) {
    for body_actor in body_actors {
        if let Some(actor) = actors.iter_mut().find(|actor| actor.id == body_actor.id) {
            for spelling in body_actor.spellings {
                if !actor.spellings.contains(&spelling) {
                    actor.spellings.push(spelling);
                }
            }
            for mention in body_actor.mentions {
                if !actor.mentions.contains(&mention) {
                    actor.mentions.push(mention);
                }
            }
        } else {
            actors.push(body_actor);
        }
    }
}

fn normalized_offsets(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut out = String::from(" ");
    let mut offsets = vec![(0, 0)];
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        let ch = if ch.is_alphanumeric() {
            ch.to_lowercase().next().unwrap_or(ch)
        } else {
            ' '
        };
        if ch == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(ch);
        offsets.extend(std::iter::repeat_n((start, end), ch.len_utf8()));
    }
    if !out.ends_with(' ') {
        out.push(' ');
        offsets.push((text.len(), text.len()));
    }
    (out, offsets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: &str, name: &str, aliases: &[&str]) -> Actor {
        Actor {
            id: id.into(),
            source_id: id.into(),
            name: name.into(),
            subtype: "organization".into(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn two_letter_and_shorter_normalized_forms_never_auto_bind() {
        let matcher = Matcher::new(vec![actor("one", "Harbor Union", &["HU", "AI"])]).unwrap();
        let article = AtlasArticleRow {
            title: "HU and AI met HU".into(),
            ..Default::default()
        };
        assert!(
            matcher.article_actors(&article).is_empty(),
            "aliases below {MIN_MATCH_CHARS} chars are metadata only"
        );
        // A bare two-character name is guarded the same way.
        let short = Matcher::new(vec![actor("one", "AI", &[])]).expect("matcher");
        assert!(short
            .article_actors(&AtlasArticleRow {
                title: "AI summit".into(),
                ..Default::default()
            })
            .is_empty());
        assert!(!matchable_alias("hu"));
        assert!(matchable_alias("hua"));
    }

    #[test]
    fn matcher_revision_is_deterministic_per_catalog_state() {
        let first = Matcher::new(vec![actor("one", "Harbor Union", &["Shared"])])
            .expect("matcher")
            .revision;
        let same = Matcher::new(vec![actor("one", "Harbor Union", &["Shared"])])
            .expect("matcher")
            .revision;
        let changed = Matcher::new(vec![actor("one", "Harbor Union", &["Shared", "Extra"])])
            .expect("matcher")
            .revision;
        assert_eq!(first, same, "same catalog state, same revision");
        assert_ne!(first, changed, "an alias change moves the revision");
    }

    #[test]
    fn curated_trump_title_binds_the_catalog_identity_without_surname_guessing() {
        let mut trump = actor("donald", "Donald Trump", &[]);
        trump.source_id = "os:Q22686".into();
        let article = AtlasArticleRow {
            title: "trump considering using nuclear weapons against iran - tucker carlson".into(),
            ..Default::default()
        };
        let matcher =
            Matcher::new(vec![trump.clone(), actor("other", "Other Trump", &[])]).unwrap();
        let found = matcher.article_actors(&article);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "donald");
        assert_eq!(found[0].mentions[0].original_text, "trump");
        let family =
            Matcher::new(vec![trump.clone(), actor("melania", "Melania Trump", &[])]).unwrap();
        let named_relative = AtlasArticleRow {
            title: "Melania Trump speaks".into(),
            ..Default::default()
        };
        let relatives = family.article_actors(&named_relative);
        assert_eq!(relatives.len(), 1);
        assert_eq!(relatives[0].id, "melania");
        let ambiguous =
            Matcher::new(vec![trump, actor("other", "Other Person", &["Trump"])]).unwrap();
        assert!(ambiguous.article_actors(&article).is_empty());
    }

    #[test]
    fn matching_respects_boundaries_ambiguity_unicode_and_original_spans() {
        let matcher = Matcher::new(vec![
            actor(
                "one",
                "Harbor Union",
                &["Harbor Union", "HU", "Shared", "München"],
            ),
            actor("two", "Other Group", &["Other Group", "Shared"]),
        ])
        .expect("matcher");
        let article = AtlasArticleRow {
            title: "Harbor-Union’s München meeting; Shared HU Other Groupish".into(),
            ..Default::default()
        };
        let found = matcher.article_actors(&article);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "one");
        assert_eq!(found[0].mentions.len(), 2);
        for m in &found[0].mentions {
            assert_eq!(&article.title[m.start_byte..m.end_byte], m.original_text);
        }
        assert_eq!(found[0].mentions[0].original_text, "Harbor-Union");
    }

    #[test]
    fn body_matches_without_nlp_and_ambiguity_stays_unbound() {
        let matcher = Matcher::new(vec![
            actor("one", "Harbor Union", &["Harbor Union", "Shared"]),
            actor("two", "Other Group", &["Other Group", "Shared"]),
        ])
        .unwrap();
        let text = "é Harbor Union, Shared, Unknown Person, Other Group";
        // NLP-off: the exact word-boundary pass binds unambiguous aliases.
        let found = matcher.body_actors(text, &[]);
        let ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["one", "two"], "unambiguous aliases bind without NLP");
        let mention = &found[0].mentions[0];
        assert_eq!(&text[mention.start_byte..mention.end_byte], "Harbor Union");
        assert_eq!(mention.field, "body");
        // With NLP, only person/organization candidates are resolved against
        // unique strong catalog aliases; ambiguous, unknown and non-matching
        // entities still stay unbound, and dedup keeps one identity per actor.
        let candidate = |form: &str, kind: &str| {
            let start = text.find(form).unwrap();
            crate::nlp::EntityCandidate {
                id: form.into(),
                type_hint: kind.into(),
                comparison_key: form.to_lowercase(),
                original_forms: vec![form.into()],
                mention_refs: vec![crate::nlp::Span {
                    start_byte: start,
                    end_byte: start + form.len(),
                }],
            }
        };
        let enriched = matcher.body_actors(
            text,
            &[
                candidate("Harbor Union", "organization"),
                candidate("Shared", "organization"),
                candidate("Unknown Person", "person"),
                candidate("Other Group", "place"),
            ],
        );
        assert_eq!(
            enriched.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["one", "two"],
            "NLP enriches but never expands beyond catalog aliases"
        );
        assert!(enriched
            .iter()
            .flat_map(|a| &a.mentions)
            .all(|m| text[m.start_byte..m.end_byte] == *m.original_text));
    }

    #[test]
    fn csv_rejects_duplicate_ids_and_malformed_aliases_without_partial_output() {
        let header = "actor_id,actor_name,actor_type,aliases,name_normalized\n";
        let row = "fixture:1,Harbor Union,organization,[],harbor union\n";
        assert_eq!(
            read(format!("{header}{row}").as_bytes())
                .expect("valid CSV")
                .len(),
            1
        );
        assert!(read(format!("{header}{row}{row}").as_bytes()).is_err());
        assert!(
            read(format!("{header}fixture:1,Union,organization,broken,union\n").as_bytes())
                .is_err()
        );
    }
}
