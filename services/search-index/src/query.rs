//! Typed, bounded query tiers; raw Tantivy query syntax is never parsed.

use fleetworks_contracts::actor_index::ActorIndexHit;
use fleetworks_contracts::actor_index::ActorIndexMutation;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::MAX_QUERY_TOKENS;
use tantivy::collector::sort_key::SortBySimilarityScore;
use tantivy::collector::sort_key::SortByString;
use tantivy::collector::TopDocs;
use tantivy::query::BooleanQuery;
use tantivy::query::Occur;
use tantivy::query::Query;
use tantivy::query::TermQuery;
use tantivy::schema::Field;
use tantivy::schema::IndexRecordOption;
use tantivy::schema::Value;
use tantivy::Order;
use tantivy::Searcher;
use tantivy::TantivyDocument;
use tantivy::Term;

use crate::schema;
use crate::schema::Fields;
use crate::IndexError;

fn term(field: Field, value: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

fn all_words(field: Field, words: &[String]) -> Box<dyn Query> {
    Box::new(BooleanQuery::new(
        words
            .iter()
            .map(|word| (Occur::Must, term(field, word)))
            .collect(),
    ))
}

pub(crate) fn record(
    searcher: &Searcher,
    fields: &Fields,
    actor_id: &str,
) -> Result<Option<ActorIndexMutation>, IndexError> {
    let query = term(fields.actor_id, actor_id);
    let rows = searcher
        .search(&*query, &TopDocs::with_limit(2).order_by_score())
        .map_err(IndexError::storage)?;
    if rows.len() > 1 {
        return Err(IndexError::unavailable("duplicate_canonical_document"));
    }
    let Some((_, address)) = rows.first() else {
        return Ok(None);
    };
    let document: TantivyDocument = searcher.doc(*address).map_err(IndexError::storage)?;
    let value = document
        .get_first(fields.record)
        .and_then(|value| value.as_str())
        .ok_or_else(|| IndexError::unavailable("invalid_projection_record"))?;
    Ok(Some(
        serde_json::from_str(value).map_err(IndexError::storage)?,
    ))
}

pub(crate) fn search(
    searcher: &Searcher,
    fields: &Fields,
    request: &ActorIndexQuery,
) -> Result<(Vec<ActorIndexHit>, bool), IndexError> {
    request.validate().map_err(IndexError::protocol)?;
    let words = schema::tokens(&request.query)?;
    if words.is_empty() || words.len() > MAX_QUERY_TOKENS {
        return Err(IndexError::invalid("query_token_limit"));
    }
    let exact = schema::normalized(&request.query);
    let mut prefix_parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let field = if index + 1 == words.len() && (2..=24).contains(&word.chars().count()) {
            fields.prefixes
        } else {
            fields.names
        };
        prefix_parts.push((Occur::Must, term(field, word)));
    }
    // Exact names/aliases are separate tiers, so BM25 can never outrank them.
    // Every tier uses the canonical ID as its stable cross-segment tie break.
    let tiers: Vec<(&str, Box<dyn Query>)> = vec![
        ("name_exact", term(fields.name_exact, &exact)),
        ("alias_exact", term(fields.alias_exact, &exact)),
        ("name_or_alias", all_words(fields.names, &words)),
        ("prefix", Box::new(BooleanQuery::new(prefix_parts))),
        ("description", all_words(fields.description, &words)),
    ];
    let mut hits: Vec<ActorIndexHit> = Vec::new();
    for (label, candidate) in tiers {
        let mut clauses = vec![
            (Occur::Must, term(fields.eligible, "yes")),
            (Occur::Must, candidate),
        ];
        for hit in &hits {
            clauses.push((Occur::MustNot, term(fields.actor_id, &hit.actor_id)));
        }
        let query = BooleanQuery::new(clauses);
        let remaining = usize::from(request.limit).saturating_sub(hits.len());
        if remaining == 0 {
            return Ok((hits, true));
        }
        let collector = TopDocs::with_limit(remaining + 1).order_by((
            (SortBySimilarityScore, Order::Desc),
            (SortByString::for_field("actor_id"), Order::Asc),
        ));
        let rows = searcher
            .search(&query, &collector)
            .map_err(IndexError::storage)?;
        let extra = rows.len() > remaining;
        for ((score, _), address) in rows.into_iter().take(remaining) {
            let document: TantivyDocument = searcher.doc(address).map_err(IndexError::storage)?;
            let value = document
                .get_first(fields.record)
                .and_then(|value| value.as_str())
                .ok_or_else(|| IndexError::unavailable("invalid_projection_record"))?;
            let mutation: ActorIndexMutation =
                serde_json::from_str(value).map_err(IndexError::storage)?;
            let projection = mutation
                .projection
                .ok_or_else(|| IndexError::unavailable("search_returned_tombstone"))?;
            hits.push(ActorIndexHit {
                actor_id: mutation.actor_id,
                revision: mutation.revision,
                projection,
                matched_field: label.to_owned(),
                score: f64::from(score),
            });
        }
        if extra || hits.len() == usize::from(request.limit) {
            return Ok((hits, true));
        }
    }
    Ok((hits, false))
}
