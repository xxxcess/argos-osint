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
    let mut table: Vec<Vec<String>> = Vec::new();
    let flush_table = |table: &mut Vec<Vec<String>>, rows: &mut Vec<MdLine>, width: usize| {
        if table.is_empty() {
            return;
        }
        rows.extend(render_table(std::mem::take(table), width));
    };
    for line in text.split('\n') {
        if let Some(rest) = line.trim().strip_prefix("```") {
            flush_table(&mut table, &mut rows, width);
            if fence.is_some() {
                fence = None;
            } else {
                fence = Some(rest.trim().to_string());
            }
            continue;
        }
        if fence.is_some() {
            flush_table(&mut table, &mut rows, width);
            rows.extend(
                hard_rows(line, width, Tone::Body)
                    .into_iter()
                    .map(|pieces| MdLine { pieces, code: true }),
            );
            blank = false;
            continue;
        }
        if line.trim().is_empty() {
            flush_table(&mut table, &mut rows, width);
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
        if let Some(cells) = table_row(trimmed) {
            if is_table_separator_cells(&cells) {
                // Keep an empty header marker so the next data rows stay in the block.
                if table.is_empty() {
                    continue;
                }
                // Separator only separates header from body; do not render it.
                continue;
            }
            table.push(cells);
            continue;
        }
        flush_table(&mut table, &mut rows, width);
        if is_rule(trimmed) {
            let bar = "─".repeat(width.clamp(1, 24));
            rows.push(md_line(vec![piece(bar, Tone::Dim)]));
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
    flush_table(&mut table, &mut rows, width);
    if rows.is_empty() {
        rows.push(MdLine {
            pieces: Vec::new(),
            code: false,
        });
    }
    rows
}

/// Parse a GFM table row: leading pipe optional, at least two cells.
fn table_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    // Bare rules like `---` are not tables.
    if is_rule(trimmed) {
        return None;
    }
    let cells: Vec<String> = trimmed
        .trim_matches(|ch| ch == '|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect();
    if cells.len() < 2 {
        return None;
    }
    // Reject prose that merely mentions a pipe once without looking tabular:
    // require content on both sides of a pipe, or a leading/trailing pipe (canonical GFM).
    let pipe_framed = trimmed.starts_with('|') || trimmed.ends_with('|');
    if !pipe_framed {
        let non_empty = cells.iter().filter(|cell| !cell.is_empty()).count();
        if non_empty < 2 {
            return None;
        }
    }
    Some(cells)
}

fn is_table_separator_cells(cells: &[String]) -> bool {
    cells.len() >= 2
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            !cell.is_empty()
                && cell
                    .chars()
                    .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
        })
}

fn render_table(rows: Vec<Vec<String>>, width: usize) -> Vec<MdLine> {
    if rows.is_empty() {
        return Vec::new();
    }
    let columns = rows.iter().map(|row| row.len()).max().unwrap_or(0).max(1);
    let mut grid: Vec<Vec<String>> = rows
        .into_iter()
        .map(|mut row| {
            while row.len() < columns {
                row.push(String::new());
            }
            row.truncate(columns);
            row
        })
        .collect();
    // Column widths from content, then shrink to fit the terminal.
    let mut widths: Vec<usize> = (0..columns)
        .map(|index| {
            grid.iter()
                .map(|row| row[index].chars().count())
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    // Separators: ` │ ` between columns → 3 chars each gap.
    let gaps = columns.saturating_sub(1) * 3;
    let available = width.saturating_sub(gaps).max(columns);
    let total: usize = widths.iter().sum();
    if total > available {
        // Shrink largest columns until they fit.
        while widths.iter().sum::<usize>() > available {
            if let Some((index, _)) = widths
                .iter()
                .enumerate()
                .filter(|(_, width)| **width > 1)
                .max_by_key(|(_, width)| *width)
            {
                widths[index] -= 1;
            } else {
                break;
            }
        }
    }
    for row in &mut grid {
        for (index, cell) in row.iter_mut().enumerate() {
            *cell = clip_cell(cell, widths[index]);
        }
    }
    let mut out = Vec::new();
    for (row_index, row) in grid.iter().enumerate() {
        let mut line = String::new();
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                line.push_str(" │ ");
            }
            let pad = widths[index].saturating_sub(cell.chars().count());
            line.push_str(cell);
            if pad > 0 {
                line.push_str(&" ".repeat(pad));
            }
        }
        let tone = if row_index == 0 {
            Tone::Heading
        } else {
            Tone::Body
        };
        // Keep each table row on one visual line; hard-clip if still over width.
        let clipped: String = line.chars().take(width).collect();
        out.push(md_line(vec![piece(clipped, tone)]));
        if row_index == 0 {
            let rule: String = widths
                .iter()
                .enumerate()
                .map(|(index, col)| {
                    let bar = "─".repeat((*col).max(1));
                    if index == 0 {
                        bar
                    } else {
                        format!("─┼─{bar}")
                    }
                })
                .collect();
            out.push(md_line(vec![piece(
                rule.chars().take(width).collect::<String>(),
                Tone::Dim,
            )]));
        }
    }
    out
}

fn clip_cell(text: &str, width: usize) -> String {
    let width = width.max(1);
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".into();
    }
    let keep: String = text.chars().take(width - 1).collect();
    format!("{keep}…")
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

    #[test]
    fn markdown_tables_align_columns_and_drop_separators() {
        let source = "| Actor | Role |\n| --- | --- |\n| Iran | State |\n| IAEA | Watchdog |\n";
        let lines = markdown_lines(source, 40);
        let text = flat(&lines);
        assert!(!text.contains("---"));
        assert!(text.contains("Actor"));
        assert!(text.contains("│"));
        assert!(text.contains("┼"));
        let header = lines[0]
            .pieces
            .iter()
            .map(|piece| piece.text.as_str())
            .collect::<String>();
        let body = lines[2]
            .pieces
            .iter()
            .map(|piece| piece.text.as_str())
            .collect::<String>();
        // Columns stay vertically aligned across header and body.
        let role_at = header.find("Role").expect("header Role");
        assert_eq!(&body[role_at..role_at + 5], "State");
        assert_eq!(lines[0].pieces[0].tone, Tone::Heading);
        // Pipe-free GFM rows also parse.
        let loose = markdown_lines("Left | Right\n--- | ---\nA | B\n", 30);
        assert!(flat(&loose).contains("Left"));
        assert!(flat(&loose).contains("A"));
        assert!(!flat(&loose).contains("---"));
    }
}
