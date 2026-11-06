//! Bounded story-family identity and permanent calibration partitions.
use crate::classifier::provider_labels::ProviderObservation;
use crate::classifier::training::{FamilyId, Split};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

pub const FAMILY_RESOLVER_VERSION: &str = "classifier-story-family-v1";
pub const FAMILY_SPLIT_VERSION: &str = "classifier-family-split-v1";
const FAMILY_CANDIDATES: usize = 100;
const FAMILY_SIGNATURE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FamilySignature {
    pub(crate) version: String,
    pub(crate) url_sha256: Option<String>,
    pub(crate) content_sha256: String,
    pub(crate) title_sha256: String,
    pub(crate) tokens: BTreeSet<String>,
    pub(crate) title_tokens: BTreeSet<String>,
    pub(crate) bands: [String; 8],
    pub(crate) observed_at: Option<i64>,
    pub(crate) shingle_count: usize,
}
#[derive(Debug, Clone)]
pub(crate) struct FamilyCandidate {
    pub(crate) family_id: FamilyId,
    pub(crate) split: Split,
    pub(crate) first_capture_seq: u64,
    pub(crate) url_sha256: Option<String>,
    pub(crate) content_sha256: String,
    pub(crate) observed_at: Option<i64>,
    pub(crate) signature: Option<FamilySignature>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FamilyResolution {
    New {
        family_id: FamilyId,
        split: Split,
    },
    Existing {
        family_id: FamilyId,
        split: Split,
        aliases: Vec<FamilyId>,
    },
    Quarantined {
        code: &'static str,
        families: Vec<FamilyId>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FamilyResolutionError {
    Cancelled,
    ObservationInvalid,
    SignatureTooLarge,
}

fn story_tokens(text: &str) -> Vec<String> {
    text.chars()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
fn permanent_split(family: &FamilyId) -> Split {
    let mut hash = Sha256::new();
    hash.update(FAMILY_SPLIT_VERSION.as_bytes());
    hash.update([0]);
    hash.update(family.as_str().as_bytes());
    let bytes = hash.finalize();
    let number = u64::from_be_bytes(
        bytes[..8]
            .try_into()
            .expect("SHA256 contains eight prefix bytes"),
    ) % 1000;
    match number {
        0..=699 => Split::Train,
        700..=849 => Split::Calibration,
        850..=949 => Split::Development,
        _ => Split::Test,
    }
}
pub(crate) fn family_signature(
    observation: &ProviderObservation,
    cancel: &AtomicBool,
) -> Result<FamilySignature, FamilyResolutionError> {
    crate::classifier::provider_labels::validate_observation(observation)
        .map_err(|_| FamilyResolutionError::ObservationInvalid)?;
    if cancel.load(Ordering::Acquire) {
        return Err(FamilyResolutionError::Cancelled);
    }
    let title = story_tokens(&observation.title);
    let description = story_tokens(&observation.description);
    let normalized_title = title.join(" ");
    let normalized_description = description.join(" ");
    let content_sha256 = crate::classifier::artifact::sha256_hex(
        crate::storage::codec::encode(&[
            FAMILY_RESOLVER_VERSION,
            &normalized_title,
            &normalized_description,
        ])
        .as_bytes(),
    );
    let title_sha256 = crate::classifier::artifact::sha256_hex(normalized_title.as_bytes());
    let url_sha256 = url::Url::parse(&observation.url)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .map(|_| {
            crate::classifier::artifact::sha256_hex(
                crate::atlas::canonical_url(&observation.url).as_bytes(),
            )
        });
    let observed_at = observation
        .published_at
        .as_deref()
        .and_then(story_time)
        .or_else(|| story_time(&observation.fetched_at));
    let ordered: Vec<_> = title.iter().chain(&description).collect();
    let mut shingles = BTreeSet::new();
    for (index, window) in ordered.windows(3).enumerate() {
        if index % 128 == 0 && cancel.load(Ordering::Acquire) {
            return Err(FamilyResolutionError::Cancelled);
        }
        shingles.insert(crate::storage::codec::encode(&[
            window[0], window[1], window[2],
        ]));
    }
    // Hash each shingle once. Independent deterministic 64-bit mixers avoid
    // 64 SHA-256 passes over the same article while retaining stable bands.
    let mut minima = [u64::MAX; 64];
    for (index, shingle) in shingles.iter().enumerate() {
        if index % 128 == 0 && cancel.load(Ordering::Acquire) {
            return Err(FamilyResolutionError::Cancelled);
        }
        let hash = Sha256::digest(shingle.as_bytes());
        let base = u64::from_be_bytes(hash[..8].try_into().expect("SHA256 prefix"));
        for (salt, minimum) in minima.iter_mut().enumerate() {
            let mut mixed = base.wrapping_add((salt as u64).wrapping_mul(0x9e3779b97f4a7c15));
            mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
            *minimum = (*minimum).min(mixed ^ (mixed >> 31));
        }
    }
    let bands = std::array::from_fn(|band| {
        let mut hash = Sha256::new();
        hash.update(FAMILY_RESOLVER_VERSION.as_bytes());
        hash.update([0]);
        hash.update((band as u64).to_be_bytes());
        for minimum in &minima[band * 8..band * 8 + 8] {
            hash.update(minimum.to_be_bytes());
        }
        format!("{:x}", hash.finalize())
    });
    let signature = FamilySignature {
        version: FAMILY_RESOLVER_VERSION.into(),
        url_sha256,
        content_sha256,
        title_sha256,
        tokens: ordered.into_iter().cloned().collect(),
        title_tokens: title.into_iter().collect(),
        bands,
        observed_at,
        shingle_count: shingles.len(),
    };
    let bytes =
        serde_json::to_vec(&signature).map_err(|_| FamilyResolutionError::SignatureTooLarge)?;
    if bytes.len() > FAMILY_SIGNATURE_BYTES {
        return Err(FamilyResolutionError::SignatureTooLarge);
    }
    Ok(signature)
}
fn story_time(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|time| time.timestamp())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|time| time.and_utc().timestamp())
                .ok()
        })
}
fn jaccard_at_least(
    left: &BTreeSet<String>,
    right: &BTreeSet<String>,
    numerator: usize,
    denominator: usize,
) -> bool {
    let intersection = left.intersection(right).count();
    let union = left.len() + right.len() - intersection;
    union > 0
        && (intersection as u128) * (denominator as u128) >= (union as u128) * (numerator as u128)
}
pub(crate) fn resolve_family(
    observation: &ProviderObservation,
    signature: &FamilySignature,
    candidates: &[FamilyCandidate],
    truncated: bool,
) -> Result<FamilyResolution, FamilyResolutionError> {
    if candidates.len() > FAMILY_CANDIDATES || truncated {
        return Ok(FamilyResolution::Quarantined {
            code: "family_candidates_truncated",
            families: candidates
                .iter()
                .map(|candidate| candidate.family_id.clone())
                .collect(),
        });
    }
    let mut matches = Vec::new();
    let mut ambiguous = Vec::new();
    for candidate in candidates {
        let same_content = candidate.content_sha256 == signature.content_sha256;
        let same_url = signature
            .url_sha256
            .as_ref()
            .zip(candidate.url_sha256.as_ref())
            .is_some_and(|(left, right)| left == right);
        let times = signature.observed_at.zip(candidate.observed_at);
        let within = times.is_some_and(|(left, right)| left.abs_diff(right) <= 72 * 60 * 60);
        if same_content {
            matches.push(candidate);
            continue;
        }
        if same_url {
            if within {
                matches.push(candidate);
                continue;
            }
            if times.is_none() {
                ambiguous.push(candidate.family_id.clone());
                continue;
            }
            let disjoint = candidate.signature.as_ref().is_some_and(|old| {
                old.tokens.is_disjoint(&signature.tokens)
                    && old.title_tokens.is_disjoint(&signature.title_tokens)
            });
            if !disjoint {
                ambiguous.push(candidate.family_id.clone());
            }
            continue;
        }
        if !within || signature.shingle_count == 0 {
            continue;
        }
        let Some(old) = candidate
            .signature
            .as_ref()
            .filter(|old| old.shingle_count > 0)
        else {
            continue;
        };
        if jaccard_at_least(&signature.tokens, &old.tokens, 9, 10)
            && jaccard_at_least(&signature.title_tokens, &old.title_tokens, 4, 5)
        {
            matches.push(candidate);
        }
    }
    if !ambiguous.is_empty() {
        return Ok(FamilyResolution::Quarantined {
            code: "ambiguous_url_reuse",
            families: ambiguous,
        });
    }
    let partitions: BTreeSet<_> = matches.iter().map(|candidate| candidate.split).collect();
    if partitions.len() > 1 {
        return Ok(FamilyResolution::Quarantined {
            code: "cross_partition_family_conflict",
            families: matches
                .iter()
                .map(|candidate| candidate.family_id.clone())
                .collect(),
        });
    }
    matches.sort_by(|left, right| {
        left.first_capture_seq
            .cmp(&right.first_capture_seq)
            .then(left.family_id.cmp(&right.family_id))
    });
    if let Some(first) = matches.first() {
        return Ok(FamilyResolution::Existing {
            family_id: first.family_id.clone(),
            split: first.split,
            aliases: matches
                .iter()
                .skip(1)
                .map(|candidate| candidate.family_id.clone())
                .collect(),
        });
    }
    let id = crate::classifier::artifact::sha256_hex(
        crate::storage::codec::encode(&[FAMILY_RESOLVER_VERSION, &observation.observation_id])
            .as_bytes(),
    );
    let family_id = FamilyId::parse(&format!("story-{id}"))
        .map_err(|_| FamilyResolutionError::ObservationInvalid)?;
    Ok(FamilyResolution::New {
        split: permanent_split(&family_id),
        family_id,
    })
}
