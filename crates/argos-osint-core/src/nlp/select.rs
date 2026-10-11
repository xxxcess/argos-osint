use super::*;
use std::collections::BTreeSet;
pub const POLICY: &str = "passage-v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub prose_hash: String,
    pub purpose: String,
    pub policy_version: String,
    pub ranges: Vec<Span>,
    pub selected_sentence_ids: Vec<String>,
    pub omitted_sentence_ids: Vec<String>,
    pub serialized_bytes: usize,
    pub selection_key: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Incomplete,
    BudgetExceeded,
    InvalidSource,
}
const SEPARATOR: &str = "\n\n[... omitted source passages ...]\n\n";
impl Selection {
    pub fn text(&self, source: &Source) -> Result<String, SelectionError> {
        if source.prose_hash != self.prose_hash || cache::hash(&source.prose) != self.prose_hash {
            return Err(SelectionError::InvalidSource);
        }
        self.ranges
            .iter()
            .map(|r| {
                r.slice(&source.prose)
                    .map(str::to_owned)
                    .ok_or(SelectionError::InvalidSource)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join(SEPARATOR))
    }
    pub fn contains(&self, source: &Source, quote: &str) -> bool {
        self.prose_hash == cache::hash(&source.prose)
            && !quote.trim().is_empty()
            && self
                .ranges
                .iter()
                .any(|r| r.slice(&source.prose).is_some_and(|s| s.contains(quote)))
    }
}
fn marker(text: &str) -> bool {
    [
        "correction",
        "corrected",
        "denied",
        "denial",
        "not ",
        "uncertain",
        "alleged",
        "disputed",
        "however",
        "retracted",
    ]
    .iter()
    .any(|m| text.contains(m))
}
/// Budget covers JSON escaped prose; callers reserve mandatory message overhead first.
/// Whole paragraphs are atomic, preserving quotations, attribution and adjacent sentences.
pub fn select(
    a: &Annotation,
    source: &Source,
    purpose: &str,
    focus: &str,
    budget: usize,
) -> Result<Selection, SelectionError> {
    if a.status != Status::Complete {
        return Err(SelectionError::Incomplete);
    }
    if !a.validate(source) {
        return Err(SelectionError::InvalidSource);
    }
    let paragraphs = spans::paragraphs(&source.prose);
    let words: Vec<_> = focus
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .collect();
    let mut scores = Vec::new();
    let mut protected = BTreeSet::new();
    for (i, p) in paragraphs.iter().enumerate() {
        let t = p.slice(&source.prose).unwrap_or("").to_lowercase();
        let correction = marker(&t);
        let mut sentence_spans: Vec<_> = a
            .sentences
            .iter()
            .filter_map(|s| {
                let start_byte = s.span.start_byte.max(p.start_byte);
                let end_byte = s.span.end_byte.min(p.end_byte);
                (start_byte < end_byte).then_some(Span {
                    start_byte,
                    end_byte,
                })
            })
            .collect();
        if sentence_spans.is_empty() {
            sentence_spans.push(*p);
        }
        let score = sentence_spans
            .iter()
            .map(|sentence| {
                let text = sentence.slice(&source.prose).unwrap_or("").to_lowercase();
                let literal = words.iter().any(|w| text.contains(w));
                let candidate = a.mentions.iter().any(|m| {
                    m.span.start_byte >= sentence.start_byte
                        && m.span.end_byte <= sentence.end_byte
                        && m.span
                            .slice(&source.prose)
                            .is_some_and(|s| words.iter().any(|w| s.to_lowercase().contains(w)))
                });
                let numeric = a
                    .numerical_spans
                    .iter()
                    .chain(a.date_candidates.iter().map(|d| &d.span))
                    .any(|s| {
                        s.start_byte >= sentence.start_byte && s.end_byte <= sentence.end_byte
                    })
                    || text.contains(['"', '“', '”']);
                usize::from(literal) * 8
                    + usize::from(candidate) * 4
                    + usize::from(marker(&text)) * 4
                    + usize::from(numeric) * 2
            })
            .sum::<usize>()
            + usize::from(i == 0 || i + 1 == paragraphs.len());
        scores.push((score, i));
        if i == 0 || correction {
            protected.insert(i);
        }
    }
    fn ranges(ps: &[Span], ids: &BTreeSet<usize>) -> Vec<Span> {
        let mut out: Vec<Span> = vec![];
        for &i in ids {
            let p = ps[i];
            if let Some(last) = out.last_mut() {
                if last.end_byte == p.start_byte {
                    last.end_byte = p.end_byte;
                    continue;
                }
            }
            out.push(p);
        }
        out
    }
    let size = |ids: &BTreeSet<usize>| {
        let t = ranges(&paragraphs, ids)
            .iter()
            .map(|r| r.slice(&source.prose).unwrap())
            .collect::<Vec<_>>()
            .join(SEPARATOR);
        serde_json::to_vec(&t).unwrap().len()
    };
    // Pair delimiters over the immutable source, including quotations opened before a match.
    let chars: Vec<_> = source.prose.char_indices().collect();
    let mut open_quotes = [None, None];
    let mut quotations = Vec::new();
    for (i, &(offset, c)) in chars.iter().enumerate() {
        let kind = if matches!(c, '"' | '“' | '”') {
            0
        } else if matches!(c, '\'' | '‘' | '’')
            && !(i > 0
                && i + 1 < chars.len()
                && chars[i - 1].1.is_alphanumeric()
                && chars[i + 1].1.is_alphanumeric())
        {
            1
        } else {
            continue;
        };
        if let Some(start_byte) = open_quotes[kind].take() {
            quotations.push(Span {
                start_byte,
                end_byte: offset + c.len_utf8(),
            });
        } else {
            open_quotes[kind] = Some(offset);
        }
    }
    for start_byte in open_quotes.into_iter().flatten() {
        quotations.push(Span {
            start_byte,
            end_byte: source.prose.len(),
        });
    }
    // Expand units to one adjacent paragraph and complete any overlapping quotation.
    let expand = |i: usize| {
        let mut unit = BTreeSet::new();
        unit.insert(i);
        if i + 1 < paragraphs.len() {
            unit.insert(i + 1);
        } else if i > 0 {
            unit.insert(i - 1);
        }
        loop {
            let old_len = unit.len();
            for quote in &quotations {
                if unit.iter().any(|&j| {
                    paragraphs[j].start_byte < quote.end_byte
                        && quote.start_byte < paragraphs[j].end_byte
                }) {
                    unit.extend(paragraphs.iter().enumerate().filter_map(|(j, p)| {
                        (p.start_byte < quote.end_byte && quote.start_byte < p.end_byte)
                            .then_some(j)
                    }));
                }
            }
            if unit.len() == old_len {
                break;
            }
        }
        unit
    };
    let mut chosen = BTreeSet::new();
    for &i in &protected {
        chosen.extend(expand(i));
    }
    if size(&chosen) > budget {
        return Err(SelectionError::BudgetExceeded);
    }
    scores.sort_by_key(|(score, i)| (std::cmp::Reverse(*score), *i));
    let all: BTreeSet<_> = (0..paragraphs.len()).collect();
    if size(&all) <= budget {
        chosen = all;
    } else {
        for (score, i) in scores {
            if score == 0 {
                continue;
            }
            let mut next = chosen.clone();
            next.extend(expand(i));
            if size(&next) <= budget {
                chosen = next;
            }
        }
    }
    let ranges = ranges(&paragraphs, &chosen);
    let mut selected = vec![];
    let mut omitted = vec![];
    for s in &a.sentences {
        if ranges
            .iter()
            .any(|r| s.span.start_byte >= r.start_byte && s.span.end_byte <= r.end_byte)
        {
            selected.push(s.id.clone());
        } else {
            omitted.push(s.id.clone());
        }
    }
    Ok(Selection {
        prose_hash: source.prose_hash.clone(),
        purpose: purpose.into(),
        policy_version: POLICY.into(),
        serialized_bytes: size(&chosen),
        selection_key: cache::key(&serde_json::json!([
            a.feature_key,
            purpose,
            focus,
            budget,
            POLICY
        ])),
        ranges,
        selected_sentence_ids: selected,
        omitted_sentence_ids: omitted,
    })
}
