//! Terminal Markdown for the Recon transcript and Brain graph summary.
//!
//! Pretty mode hides the source markers, matching Grok Build's agent messages:
//! headings, emphasis, inline code, links, lists, quotes, and fenced code.

use ratatui::style::{Modifier, Style};

use super::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Body,
    Dim,
    Accent,
    Bold,
    Italic,
    Strike,
    Code,
    Heading,
    Link,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub tone: Tone,
}

/// One visual row. `code` is set only for fenced blocks, so inline code keeps the text background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdLine {
    pub pieces: Vec<Piece>,
    pub code: bool,
}

pub fn style(tone: Tone) -> Style {
    match tone {
        Tone::Body => theme::text(),
        Tone::Dim => theme::dim(),
        Tone::Accent | Tone::Heading => theme::accent().add_modifier(Modifier::BOLD),
        Tone::Bold => theme::text().add_modifier(Modifier::BOLD),
        Tone::Italic => theme::text().add_modifier(Modifier::ITALIC),
        Tone::Strike => theme::dim().add_modifier(Modifier::CROSSED_OUT),
        Tone::Code => theme::accent().add_modifier(Modifier::BOLD),
        Tone::Link => theme::accent().add_modifier(Modifier::UNDERLINED),
        Tone::Warn => theme::warn(),
        Tone::Error => theme::error(),
    }
}

pub fn markdown_lines(text: &str, width: usize) -> Vec<MdLine> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut fence: Option<String> = None;
    let mut blank = false;
    for line in text.split('\n') {
        if let Some(rest) = line.trim().strip_prefix("```") {
            if fence.is_some() {
                fence = None;
            } else {
                fence = Some(rest.trim().to_string());
            }
            continue;
        }
        if fence.is_some() {
            rows.extend(
                hard_rows(line, width, Tone::Body)
                    .into_iter()
                    .map(|pieces| MdLine { pieces, code: true }),
            );
            blank = false;
            continue;
        }
        if line.trim().is_empty() {
            if !blank && !rows.is_empty() {
                rows.push(MdLine {
                    pieces: Vec::new(),
                    code: false,
                });
                blank = true;
            }
            continue;
        }
        blank = false;
        let trimmed = line.trim();
        if is_rule(trimmed) {
            let bar = "─".repeat(width.clamp(1, 24));
            rows.push(md_line(vec![piece(bar, Tone::Dim)]));
            continue;
        }
        if is_table_separator(trimmed) {
            continue;
        }
        if trimmed.starts_with('|') && trimmed.matches('|').count() >= 2 {
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            let joined = cells.join(" │ ");
            rows.extend(
                wrap_pieces(&inline(&joined), width)
                    .into_iter()
                    .map(md_line),
            );
            continue;
        }
        if let Some(marks) = heading_marks(trimmed) {
            let content = paint(inline(trimmed[marks..].trim()), Tone::Heading);
            rows.extend(wrap_pieces(&content, width).into_iter().map(md_line));
            continue;
        }
        if let Some(quoted) = trimmed.strip_prefix('>') {
            let content = inline(quoted.trim());
            rows.extend(
                with_prefix(&content, "│ ", "│ ", width, Tone::Dim)
                    .into_iter()
                    .map(md_line),
            );
            continue;
        }
        if let Some(item) = list_item(trimmed) {
            rows.extend(
                with_prefix(
                    &inline(item.body),
                    &item.marker,
                    &" ".repeat(item.marker.chars().count()),
                    width,
                    Tone::Dim,
                )
                .into_iter()
                .map(md_line),
            );
            continue;
        }
        rows.extend(
            wrap_pieces(&inline(trimmed), width)
                .into_iter()
                .map(md_line),
        );
    }
    if rows.is_empty() {
        rows.push(MdLine {
            pieces: Vec::new(),
            code: false,
        });
    }
    rows
}

fn md_line(pieces: Vec<Piece>) -> MdLine {
    MdLine {
        pieces,
        code: false,
    }
}

pub fn plain_lines(text: &str, width: usize, indent: usize) -> Vec<Vec<Piece>> {
    let width = width.max(1);
    let indent = indent.min(width.saturating_sub(1));
    let inner = width - indent;
    let pad = " ".repeat(indent);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            rows.push(Vec::new());
            continue;
        }
        for hard in hard_rows(line, inner, Tone::Dim) {
            let mut pieces = vec![piece(pad.clone(), Tone::Dim)];
            pieces.extend(hard);
            rows.push(pieces);
        }
    }
    rows
}

/// User prompt band: a `❯ ` arrow on the first visual line, spaces on the rest.
pub fn user_lines(text: &str, width: usize) -> Vec<Vec<Piece>> {
    let width = width.max(1);
    let prefix = "❯ ";
    let prefix_width = prefix.chars().count();
    let inner = width.saturating_sub(prefix_width).max(1);
    let source = if text.is_empty() { " " } else { text };
    let mut rows = Vec::new();
    for (index, line) in source.split('\n').enumerate() {
        let wrapped = if line.is_empty() {
            vec![Vec::new()]
        } else {
            wrap_pieces(&[piece(line, Tone::Body)], inner)
        };
        for (wrap_index, content) in wrapped.into_iter().enumerate() {
            let mark = if index == 0 && wrap_index == 0 {
                prefix
            } else {
                "  "
            };
            let mut pieces = vec![piece(mark, Tone::Accent)];
            pieces.extend(content);
            rows.push(pieces);
        }
    }
    if rows.is_empty() {
        rows.push(vec![piece(prefix, Tone::Accent)]);
    }
    rows
}

fn piece(text: impl Into<String>, tone: Tone) -> Piece {
    Piece {
        text: text.into(),
        tone,
    }
}

fn paint(mut pieces: Vec<Piece>, tone: Tone) -> Vec<Piece> {
    for item in &mut pieces {
        if item.tone == Tone::Body {
            item.tone = tone;
        }
    }
    pieces
}

fn heading_marks(line: &str) -> Option<usize> {
    let marks = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&marks) && line.chars().nth(marks) == Some(' ') {
        Some(marks + 1)
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let chars: Vec<char> = line.chars().filter(|ch| !ch.is_whitespace()).collect();
    chars.len() >= 3
        && chars
            .iter()
            .all(|ch| *ch == '-' || *ch == '*' || *ch == '_')
}

fn is_table_separator(line: &str) -> bool {
    let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
    cells.len() >= 2
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            !cell.is_empty()
                && cell
                    .chars()
                    .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
        })
}

struct ListItem<'a> {
    marker: String,
    body: &'a str,
}

fn list_item(line: &str) -> Option<ListItem<'_>> {
    let bytes = line.as_bytes();
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some(ListItem {
            marker: "• ".into(),
            body: rest,
        });
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && digits <= 3 {
        let rest = &line[digits..];
        if let Some(body) = rest.strip_prefix(". ") {
            return Some(ListItem {
                marker: format!("{}. ", &line[..digits]),
                body,
            });
        }
    }
    None
}

fn inline(input: &str) -> Vec<Piece> {
    let mut rows = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        if let Some((body, after)) = wrapped(rest, "**") {
            rows.extend(paint(inline(body), Tone::Bold));
            rest = after;
            continue;
        }
        if let Some((body, after)) = wrapped(rest, "~~") {
            rows.extend(paint(inline(body), Tone::Strike));
            rest = after;
            continue;
        }
        if rest.starts_with('`') {
            if let Some(end) = rest[1..].find('`') {
                rows.push(piece(&rest[1..1 + end], Tone::Code));
                rest = &rest[2 + end..];
                continue;
            }
        }
        if rest.starts_with('*') && !rest.starts_with("**") {
            if let Some((body, after)) = wrapped(rest, "*") {
                rows.extend(paint(inline(body), Tone::Italic));
                rest = after;
                continue;
            }
        }
        if let Some((text, after)) = link(rest) {
            rows.push(piece(text, Tone::Link));
            rest = after;
            continue;
        }
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(_, ch)| matches!(ch, '*' | '`' | '~' | '['))
            .map(|(index, _)| index)
            .unwrap_or(rest.len());
        rows.push(piece(&rest[..next], Tone::Body));
        rest = &rest[next..];
    }
    rows
}

fn wrapped<'a>(input: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let body = input.strip_prefix(marker)?;
    let end = body.find(marker)?;
    if end == 0 {
        return None;
    }
    Some((&body[..end], &body[end + marker.len()..]))
}

fn link(input: &str) -> Option<(&str, &str)> {
    let body = input.strip_prefix('[')?;
    let close = body.find("](")?;
    let text = &body[..close];
    let after = &body[close + 2..];
    let end = after.find(')')?;
    if text.is_empty() {
        return None;
    }
    Some((text, &after[end + 1..]))
}

fn with_prefix(
    content: &[Piece],
    first: &str,
    next: &str,
    width: usize,
    tone: Tone,
) -> Vec<Vec<Piece>> {
    let inner = width.saturating_sub(first.chars().count()).max(1);
    let wrapped = if content.is_empty() {
        vec![Vec::new()]
    } else {
        wrap_pieces(content, inner)
    };
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mut pieces = vec![piece(if index == 0 { first } else { next }, tone)];
            pieces.extend(line);
            pieces
        })
        .collect()
}

fn wrap_pieces(pieces: &[Piece], width: usize) -> Vec<Vec<Piece>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut column = 0usize;
    for piece in pieces {
        for word in split_words(&piece.text) {
            if word.chars().all(char::is_whitespace) {
                if column > 0 && column < width {
                    push_text(rows.last_mut().unwrap(), " ", piece.tone);
                    column += 1;
                }
                continue;
            }
            let word_width = word.chars().count();
            if word_width > width {
                if column > 0 {
                    rows.push(Vec::new());
                    column = 0;
                }
                let mut buf = String::new();
                for ch in word.chars() {
                    if column == width {
                        push_text(rows.last_mut().unwrap(), &buf, piece.tone);
                        rows.push(Vec::new());
                        buf.clear();
                        column = 0;
                    }
                    buf.push(ch);
                    column += 1;
                }
                if !buf.is_empty() {
                    push_text(rows.last_mut().unwrap(), &buf, piece.tone);
                }
                continue;
            }
            if column > 0 && column + word_width > width {
                rows.push(Vec::new());
                column = 0;
            }
            push_text(rows.last_mut().unwrap(), word, piece.tone);
            column += word_width;
        }
    }
    if rows.len() == 1 && rows[0].is_empty() {
        return rows;
    }
    rows.retain(|row| !row.is_empty());
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

fn push_text(row: &mut Vec<Piece>, text: &str, tone: Tone) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = row.last_mut() {
        if last.tone == tone {
            last.text.push_str(text);
            return;
        }
    }
    row.push(piece(text, tone));
}

fn split_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0usize;
    let mut in_space = None;
    for (index, ch) in text.char_indices() {
        let space = ch.is_whitespace();
        if in_space != Some(space) {
            if index > start {
                words.push(&text[start..index]);
            }
            start = index;
            in_space = Some(space);
        }
    }
    if start < text.len() {
        words.push(&text[start..]);
    }
    words
}

fn hard_rows(text: &str, width: usize, tone: Tone) -> Vec<Vec<Piece>> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![Vec::new()];
    }
    let mut rows = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if buf.chars().count() == width {
            rows.push(vec![piece(std::mem::take(&mut buf), tone)]);
        }
        buf.push(ch);
    }
    if !buf.is_empty() {
        rows.push(vec![piece(buf, tone)]);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &[MdLine]) -> String {
        lines
            .iter()
            .map(|line| {
                line.pieces
                    .iter()
                    .map(|piece| piece.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pretty_markdown_hides_markers() {
        let lines = markdown_lines(
            "# Who\n\n**Ada** wrote `notes` and [mail](https://example.org).\n\n- one\n\n> quoted\n\n```\nline\n```\n",
            48,
        );
        let text = flat(&lines);
        assert!(text.contains("Who"));
        assert!(!text.contains("# Who"));
        assert!(text.contains("Ada"));
        assert!(lines
            .iter()
            .flat_map(|line| &line.pieces)
            .any(|piece| { piece.text.contains("Ada") && piece.tone == Tone::Bold }));
        assert!(lines.iter().any(|line| {
            !line.code
                && line
                    .pieces
                    .iter()
                    .any(|piece| piece.text == "notes" && piece.tone == Tone::Code)
        }));
        assert!(lines
            .iter()
            .any(|line| { line.code && line.pieces.iter().any(|piece| piece.text == "line") }));
        assert!(text.contains("mail"));
        assert!(!text.contains("https://example.org"));
        assert!(text.contains("• one"));
        assert!(text.contains("│ quoted"));
        assert!(text.contains("line"));
        assert!(!text.contains("```"));
    }

    #[test]
    fn user_prompt_keeps_the_arrow_on_the_first_line_only() {
        let lines = user_lines("who is ada lovelace", 12);
        assert!(lines[0][0].text.starts_with("❯"));
        assert!(lines.len() > 1);
        assert_eq!(lines[1][0].text, "  ");
    }

    #[test]
    fn fenced_code_is_marked_and_plus_lists_use_a_bullet() {
        let lines = markdown_lines("use `dig` here\n\n```\nname\n```\n\n+ second", 40);
        assert!(lines.iter().any(|line| {
            !line.code
                && line
                    .pieces
                    .iter()
                    .any(|piece| piece.text == "dig" && piece.tone == Tone::Code)
        }));
        assert!(lines.iter().any(|line| {
            line.code
                && line
                    .pieces
                    .iter()
                    .any(|piece| piece.text == "name" && piece.tone == Tone::Body)
        }));
        assert!(flat(&lines).contains("• second"));
        assert!(!flat(&lines).contains("```"));
    }
}
