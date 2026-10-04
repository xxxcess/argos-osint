//! Validate retrieved article bodies before committing them as full content.

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyQuality {
    Complete,
    Partial,
    Uncertain,
    Unavailable,
}

impl BodyQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Uncertain => "uncertain",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BodyValidation {
    pub quality: BodyQuality,
    pub rationale: String,
    pub cleaned_markdown: String,
    pub content_hash: String,
}

/// Markers that indicate the page is not the article body.
const BLOCK_MARKERS: &[&str] = &[
    "enable javascript",
    "please enable cookies",
    "cookie consent",
    "verify you are human",
    "access denied",
    "captcha",
    "sign in to continue",
    "log in to continue",
    "subscribe to read",
    "subscribe to continue",
    "create a free account",
    "paywall",
    "you have reached your limit",
];

pub fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn validate_article_body(
    markdown: &str,
    expected_title: &str,
    expected_url: &str,
) -> BodyValidation {
    let cleaned = clean_markdown(markdown);
    let lower = cleaned.to_ascii_lowercase();
    let hash = content_hash(&cleaned);

    if cleaned.trim().is_empty() {
        return BodyValidation {
            quality: BodyQuality::Unavailable,
            rationale: "empty body".into(),
            cleaned_markdown: String::new(),
            content_hash: hash,
        };
    }

    for marker in BLOCK_MARKERS {
        if lower.contains(marker) && word_count(&cleaned) < 120 {
            return BodyValidation {
                quality: BodyQuality::Unavailable,
                rationale: format!("blocked or interstitial page ({marker})"),
                cleaned_markdown: cleaned,
                content_hash: hash,
            };
        }
    }

    let words = word_count(&cleaned);
    let title = expected_title.trim();
    let title_hit = !title.is_empty()
        && title
            .split_whitespace()
            .filter(|t| t.len() > 3)
            .take(4)
            .any(|token| lower.contains(&token.to_ascii_lowercase()));

    if looks_like_search_snippet(&cleaned) {
        return BodyValidation {
            quality: BodyQuality::Unavailable,
            rationale: "search snippet, not full article".into(),
            cleaned_markdown: cleaned,
            content_hash: hash,
        };
    }

    // Short complete articles are allowed when the title matches; snippets alone are not.
    if words < 20 {
        return BodyValidation {
            quality: BodyQuality::Unavailable,
            rationale: format!("too short for an article body ({words} words)"),
            cleaned_markdown: cleaned,
            content_hash: hash,
        };
    }

    if words < 120 && title_hit {
        return BodyValidation {
            quality: BodyQuality::Complete,
            rationale: "short but complete article".into(),
            cleaned_markdown: cleaned,
            content_hash: hash,
        };
    }

    if words < 120 {
        return BodyValidation {
            quality: BodyQuality::Partial,
            rationale: "short extract without clear title match".into(),
            cleaned_markdown: cleaned,
            content_hash: hash,
        };
    }

    if !title_hit && !expected_url.is_empty() {
        return BodyValidation {
            quality: BodyQuality::Uncertain,
            rationale: "usable text without strong title identity match".into(),
            cleaned_markdown: cleaned,
            content_hash: hash,
        };
    }

    BodyValidation {
        quality: BodyQuality::Complete,
        rationale: "validated article body".into(),
        cleaned_markdown: cleaned,
        content_hash: hash,
    }
}

fn clean_markdown(raw: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in raw.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank += 1;
            if blank <= 2 {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        // Drop obvious nav chrome lines.
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("skip to ")
            || lower == "menu"
            || lower == "navigation"
            || lower.starts_with("advertisement")
        {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    out.trim().to_string()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn looks_like_search_snippet(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() >= 4 && lines.len() <= 12 {
        let short = lines.iter().filter(|l| word_count(l) <= 28).count();
        return short * 100 / lines.len() >= 80;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_complete_article_passes() {
        let body = "Geneva talks resume.\n\nDiplomats from France and Germany met today to discuss the ceasefire proposal after overnight shelling near the border crossing.";
        let v = validate_article_body(body, "Geneva talks resume", "https://example.com/a");
        assert_eq!(v.quality, BodyQuality::Complete);
    }

    #[test]
    fn paywall_is_unavailable() {
        let body = "Subscribe to continue reading this article. Create a free account.";
        let v = validate_article_body(body, "Secret deal", "https://example.com/a");
        assert_eq!(v.quality, BodyQuality::Unavailable);
    }

    #[test]
    fn empty_is_unavailable() {
        let v = validate_article_body("   ", "Title", "https://example.com");
        assert_eq!(v.quality, BodyQuality::Unavailable);
    }
}
