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
    let mut cleaned_lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if cleaned_lines
                .last()
                .map(|l: &String| !l.is_empty())
                .unwrap_or(false)
            {
                cleaned_lines.push(String::new());
            }
            continue;
        }
        let lower = trimmed.trim().to_ascii_lowercase();
        if lower.starts_with("skip to ")
            || lower == "menu"
            || lower == "navigation"
            || lower.starts_with("advertisement")
        {
            continue;
        }
        cleaned_lines.push(trimmed.to_string());
    }

    // Strip leading publisher site-wide navigation lists / link rosters.
    let mut start_idx = 0;
    while start_idx < cleaned_lines.len() {
        let line = cleaned_lines[start_idx].trim();
        if line.is_empty() {
            start_idx += 1;
            continue;
        }
        let is_nav_bullet = line.starts_with('•')
            || line.starts_with('*')
            || line.starts_with('-')
            || line.starts_with("·");
        let is_short_link = (line.starts_with('[') && line.contains("](") && line.len() < 50)
            || (is_nav_bullet && line.len() < 45);
        if is_short_link {
            let mut end_nav = start_idx + 1;
            while end_nav < cleaned_lines.len() {
                let next_line = cleaned_lines[end_nav].trim();
                if next_line.is_empty() {
                    end_nav += 1;
                    continue;
                }
                let next_is_bullet = next_line.starts_with('•')
                    || next_line.starts_with('*')
                    || next_line.starts_with('-')
                    || next_line.starts_with("·");
                let next_is_short = (next_line.starts_with('[')
                    && next_line.contains("](")
                    && next_line.len() < 50)
                    || (next_is_bullet && next_line.len() < 45);
                if next_is_short {
                    end_nav += 1;
                } else {
                    break;
                }
            }
            let count = cleaned_lines[start_idx..end_nav]
                .iter()
                .filter(|l| !l.trim().is_empty())
                .count();
            if count >= 3 {
                start_idx = end_nav;
                continue;
            }
        }
        break;
    }

    cleaned_lines[start_idx..].join("\n").trim().to_string()
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

    #[test]
    fn strips_leading_sitewide_nav_link_roster() {
        let noisy = "• India Today\n• Aaj Tak\n• Business Today\n• Cosmopolitan\n\nDonald Trump makes another Nobel Peace Prize pitch.\nDonald Trump is making another pitch for the Nobel Peace Prize.";
        let v = validate_article_body(
            noisy,
            "Trump makes another Nobel Peace Prize pitch",
            "https://example.com",
        );
        assert!(!v.cleaned_markdown.contains("India Today"));
        assert!(!v.cleaned_markdown.contains("Aaj Tak"));
        assert!(v
            .cleaned_markdown
            .contains("Donald Trump is making another pitch"));
    }
}
