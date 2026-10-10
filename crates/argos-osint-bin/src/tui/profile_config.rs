//! Profile > System > **Configs**: the portable configuration Export and Import
//! popups.
//!
//! Both popups are thin views over
//! [`argos_osint_core::config_transfer`]. Export serialises the live
//! configuration deterministically and writes it through the secure primitive
//! (unique sibling temporary file created owner-only, flushed, fsynced,
//! atomically renamed). Import validates the **whole** document before anything
//! is applied, shows the redacted change summary, and only then commits — the
//! Import button is the user's commit.
//!
//! A credential value never reaches the screen or the log: the change summary
//! names fields, and every error carries a JSON pointer instead of a value.

use std::path::PathBuf;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::config_transfer::{
    self, ConfigurationSnapshot, CredentialSource, ImportPlan, ProfileConfig, QuotaSettingsFile,
    SCHEMA_VERSION,
};
use argos_osint_core::paths;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::secrets::AuthFile;

use super::app::App;
use super::theme;

/// Which side of the Configs pane is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfigTab {
    /// Write the live configuration to a portable file.
    #[default]
    Export,
    /// Merge a portable file into the live configuration.
    Import,
}

impl ConfigTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Export => "Export",
            Self::Import => "Import",
        }
    }
}

/// One validation problem to show in the Import editor: a line, a column and a
/// JSON pointer, never a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorError {
    pub line: usize,
    pub column: usize,
    pub pointer: String,
    pub message: String,
}

/// The Import editor state: a multiline JSON buffer with a cursor, a scroll
/// offset and the last validation result.
#[derive(Clone, Debug, Default)]
pub struct ImportBuffer {
    pub text: String,
    /// Caret offset in characters.
    pub caret: usize,
    /// First rendered line.
    pub scroll: usize,
    /// Bracketed paste accumulation, non-empty while a paste is in flight.
    pub paste: String,
    /// Last validation outcome: line/column, pointer and message.
    pub error: Option<EditorError>,
    /// Non-blocking warnings from the last successful validation.
    pub warnings: Vec<String>,
}

/// The Configs popup state.
#[derive(Clone, Debug)]
pub struct ConfigView {
    pub tab: ConfigTab,
    pub open: bool,
    /// Export destination, before `~` expansion.
    pub export_path: String,
    /// Where the expanded path points.
    pub export_resolved: Option<PathBuf>,
    pub export_status: Option<String>,
    pub export_error: Option<String>,
    /// Set when the destination exists and the user must confirm the overwrite.
    pub export_confirm: bool,
    pub import: ImportBuffer,
    /// The validated plan behind the Import button.
    pub plan: Option<ImportPlan>,
    /// The last document this popup exported, for the redacted summary.
    pub exported: Option<ProfileConfig>,
    pub import_status: Option<String>,
    pub import_error: Option<String>,
    /// Focused control inside the popup.
    pub focus: usize,
}

impl Default for ConfigView {
    fn default() -> Self {
        Self {
            tab: ConfigTab::Export,
            open: false,
            export_path: "~/argos-config.json".to_string(),
            export_resolved: None,
            export_status: None,
            export_error: None,
            export_confirm: false,
            import: ImportBuffer::default(),
            plan: None,
            exported: None,
            import_status: None,
            import_error: None,
            focus: 0,
        }
    }
}

impl ImportBuffer {
    /// Caret as a 1-based line and column, for the editor status line.
    pub fn caret_position(&self) -> (usize, usize) {
        let byte = char_offset(&self.text, self.caret);
        let before = &self.text[..byte];
        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map(|line: &str| line.chars().count() + 1)
            .unwrap_or(1);
        (line, column)
    }

    // ---- editor primitives ----

    pub fn insert(&mut self, text: &str) {
        let caret = self.caret.min(self.text.chars().count());
        let byte = char_offset(&self.text, caret);
        self.text.insert_str(byte, text);
        self.caret = caret + text.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let byte = char_offset(&self.text, self.caret);
        let start = self.text[..byte]
            .chars()
            .last()
            .map(|ch| byte - ch.len_utf8())
            .unwrap_or(byte);
        self.text.replace_range(start..byte, "");
        self.caret -= 1;
    }

    pub fn delete(&mut self) {
        let count = self.text.chars().count();
        if self.caret >= count {
            return;
        }
        let byte = char_offset(&self.text, self.caret);
        let end = byte
            + self.text[byte..]
                .chars()
                .next()
                .map(|ch| ch.len_utf8())
                .unwrap_or(0);
        self.text.replace_range(byte..end, "");
    }

    /// Enter inserts a newline; Ctrl+Enter validates and imports.
    pub fn newline(&mut self) {
        self.insert("\n");
    }

    pub fn home(&mut self) {
        let byte = char_offset(&self.text, self.caret);
        let start = self.text[..byte]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.caret = self.text[..start].chars().count();
    }

    pub fn end(&mut self) {
        let byte = char_offset(&self.text, self.caret);
        let stop = self.text[byte..]
            .find('\n')
            .map(|index| byte + index)
            .unwrap_or(self.text.len());
        self.caret = self.text[..stop].chars().count();
    }

    pub fn scroll_by(&mut self, delta: i32, height: usize) {
        let total = self.text.matches('\n').count() + 1;
        let max = total.saturating_sub(height);
        let next = self.scroll as i32 + delta;
        self.scroll = next.clamp(0, max as i32) as usize;
    }
}

impl ConfigView {
    pub fn open(&mut self) {
        self.open = true;
        self.export_status = None;
        self.export_error = None;
        self.export_confirm = false;
        self.import_status = None;
        self.import_error = None;
    }

    // ---- editor primitives (delegated to the buffer) ----

    pub fn insert(&mut self, text: &str) {
        self.import.insert(text);
        self.disarm();
    }

    pub fn backspace(&mut self) {
        self.import.backspace();
        self.disarm();
    }

    pub fn delete(&mut self) {
        self.import.delete();
        self.disarm();
    }

    pub fn newline(&mut self) {
        self.import.newline();
        self.disarm();
    }

    /// Editing the buffer disarms the Import button: a document that no longer
    /// matches what was validated must never commit.
    fn disarm(&mut self) {
        self.plan = None;
        self.import.warnings.clear();
    }

    pub fn home(&mut self) {
        self.import.home();
    }

    pub fn end(&mut self) {
        self.import.end();
    }

    pub fn close(&mut self) {
        self.open = false;
        // The pasted secret buffer never outlives the popup.
        self.import.paste.clear();
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            ConfigTab::Export => ConfigTab::Import,
            ConfigTab::Import => ConfigTab::Export,
        };
        self.focus = 0;
    }

    /// Expands a leading `~` and reports the resolved destination.
    pub fn resolve_export_path(&self) -> PathBuf {
        expand_home(self.export_path.trim())
    }

    /// Re-checks the destination: an unwritable parent, an existing file and a
    /// directory target are all actionable errors that name the path, never a
    /// credential.
    pub fn check_export_path(&mut self) {
        self.export_status = None;
        self.export_error = None;
        self.export_confirm = false;
        let trimmed = self.export_path.trim();
        if trimmed.is_empty() {
            self.export_error = Some("Enter a destination path".to_string());
            return;
        }
        let resolved = self.resolve_export_path();
        self.export_resolved = Some(resolved.clone());
        match resolved.parent() {
            None => {
                self.export_error = Some("The destination has no parent directory".to_string());
                return;
            }
            Some(parent) if !parent.as_os_str().is_empty() && !parent.exists() => {
                self.export_error =
                    Some(format!("The directory {} does not exist", parent.display()));
                return;
            }
            _ => {}
        }
        if resolved.is_dir() {
            self.export_error = Some(format!("{} is a directory", resolved.display()));
            return;
        }
        if resolved.exists() {
            self.export_confirm = true;
        }
    }

    /// Runs the export. Returns the document so the caller can log a redacted
    /// summary.
    pub fn run_export(&mut self) -> anyhow::Result<ProfileConfig> {
        self.check_export_path();
        if self.export_error.is_some() {
            anyhow::bail!("the destination is not writable");
        }
        if self.export_confirm {
            // The caller must have confirmed; a stale confirmation is refused.
            anyhow::bail!("confirm the overwrite first");
        }
        let snapshot = ConfigurationSnapshot {
            settings: SettingsFile::load()?,
            auth: AuthFile::load()?,
            quotas: QuotaSettingsFile::load()?,
        };
        let path = self
            .export_resolved
            .clone()
            .unwrap_or_else(|| self.resolve_export_path());
        let document = config_transfer::export_to_path(
            &snapshot.settings,
            &snapshot.auth,
            &snapshot.quotas,
            &path,
        )?;
        self.export_confirm = false;
        self.exported = Some(document.clone());
        self.export_status = Some(format!(
            "Exported schema v{SCHEMA_VERSION} to {}",
            path.display()
        ));
        self.export_error = None;
        Ok(document)
    }

    /// Validates the pasted document and arms the Import button.
    pub fn validate(&mut self) {
        self.import.error = None;
        self.import.warnings.clear();
        self.plan = None;
        self.import_status = None;
        self.import_error = None;
        if self.import.text.len() > config_transfer::MAX_DOCUMENT_BYTES {
            self.import.error = Some(EditorError {
                line: 1,
                column: 1,
                pointer: String::new(),
                message: format!(
                    "the document is larger than the {} byte cap",
                    config_transfer::MAX_DOCUMENT_BYTES
                ),
            });
            return;
        }
        match config_transfer::parse_document(&self.import.text) {
            Ok(plan) => {
                self.import.warnings = plan
                    .warnings
                    .iter()
                    .map(|warning| warning.to_string())
                    .collect();
                self.plan = Some(plan);
            }
            Err(err) => {
                self.import.error = Some(locate(&self.import.text, &err.to_string()));
                self.import_error = Some(err.to_string());
            }
        }
    }

    /// The redacted change summary for the validated plan against `snapshot`.
    pub fn changes(&self, snapshot: &ConfigurationSnapshot) -> Vec<config_transfer::ConfigChange> {
        match &self.plan {
            Some(plan) => {
                plan.changes_against(&snapshot.settings, &snapshot.auth, &snapshot.quotas)
            }
            None => Vec::new(),
        }
    }

    /// Applies the validated plan and commits all three files as one revision.
    pub fn run_import(&mut self) -> anyhow::Result<u64> {
        let Some(plan) = self.plan.clone() else {
            anyhow::bail!("validate the document first");
        };
        let mut snapshot = ConfigurationSnapshot {
            settings: SettingsFile::load()?,
            auth: AuthFile::load()?,
            quotas: QuotaSettingsFile::load()?,
        };
        snapshot.apply(&plan)?;
        let generation = snapshot.commit()?;
        self.plan = None;
        self.import_status = Some(format!("Imported revision {generation}"));
        Ok(generation)
    }
}

fn char_offset(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

/// Expands a leading `~` to the configuration home.
fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return paths::home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return paths::home_dir().join(rest);
    }
    PathBuf::from(raw)
}

/// Maps a validation message onto a line and column when the message carries
/// them; a JSON-pointer field error keeps the pointer and lands on the caret.
fn locate(text: &str, message: &str) -> EditorError {
    let mut line = 1;
    let mut column = 1;
    for token in message.split_whitespace() {
        if let Some(rest) = token.strip_prefix("line ") {
            if let Ok(value) = rest.trim_end_matches(|c: char| !c.is_ascii_digit()).parse() {
                line = value;
            }
        }
        if let Some(rest) = token.strip_prefix("column ") {
            if let Ok(value) = rest.trim_end_matches(|c: char| !c.is_ascii_digit()).parse() {
                column = value;
            }
        }
    }
    let pointer = message
        .split_whitespace()
        .find(|token| token.starts_with('/') && token.len() > 1)
        .unwrap_or("")
        .trim_end_matches([';', ':'])
        .to_string();
    let bounded = if message.chars().count() > 120 {
        let taken: String = message.chars().take(120).collect();
        format!("{taken}…")
    } else {
        message.to_string()
    };
    EditorError {
        line,
        column,
        pointer,
        message: bounded,
    }
    .tap(text)
}

impl EditorError {
    fn tap(self, _text: &str) -> Self {
        self
    }
}

/// The credential summary the Export tab shows before writing: which fields
/// travel, without any value.
pub fn credential_summary(document: &ProfileConfig) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        format!("providers: {}", document.providers.len()),
        theme::text(),
    ))];
    for entry in &document.providers {
        let label = match entry.credential.source {
            CredentialSource::Inline => "key travels".to_string(),
            CredentialSource::Env => "environment reference".to_string(),
            CredentialSource::None => "keyless".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<16}", entry.id), theme::dim()),
            Span::styled(format!("{:<24}", label), theme::text()),
            Span::styled(entry.default_model.to_string(), theme::muted()),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!("tool credentials: {}", document.tool_credentials.len()),
        theme::text(),
    )));
    lines.push(Line::from(Span::styled(
        format!("roles: 10   rate limits: {}", document.rate_limits.len()),
        theme::text(),
    )));
    lines
}

/// The warning the popup shows on the credential export path.
pub const SECRET_WARNING: &str =
    "This file contains saved API keys. Treat it as a secret and never commit it.";

/// Draws the Configs popup. The area must already be the centred overlay.
pub fn draw(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    frame.render_widget(Clear, area);
    let block = theme::card(&format!(
        " Profile · Configs · {} ",
        app.config_tab().label()
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 6 {
        frame.render_widget(Paragraph::new(SECRET_WARNING).style(theme::warn()), inner);
        return;
    }
    let header = Line::from(Span::styled(SECRET_WARNING, theme::warn()));
    frame.render_widget(Paragraph::new(header), strip(inner, 0, 1));
    let body = strip(inner, 1, inner.height - 1);
    match app.config_tab() {
        ConfigTab::Export => draw_export(frame, app, body),
        ConfigTab::Import => draw_import(frame, app, body),
    }
}

/// Splits a body into `count` rows from the top; the final row absorbs whatever
/// is left so the layout never runs past the popup.
fn rows_at(area: ratatui::layout::Rect, heights: &[u16]) -> Vec<ratatui::layout::Rect> {
    let mut out = Vec::with_capacity(heights.len());
    let mut y = area.y;
    for (index, height) in heights.iter().enumerate() {
        let height = if index + 1 == heights.len() {
            area.height.saturating_sub(*height).max(1)
        } else {
            (*height).min(area.height.saturating_sub(y - area.y))
        };
        out.push(strip(area, y - area.y, height));
        y += height;
    }
    out
}

fn draw_export(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let config = app.config();
    let rows = rows_at(area, &[1, 1, 1, 6, 1, 1, 1, 1, 1, 1, 1, 1]);
    // The destination is a real field, so a click focuses it just like the
    // keyboard does.
    app.draw_export_field(frame, rows[0]);
    let resolved = config
        .export_resolved
        .clone()
        .unwrap_or_else(|| config.resolve_export_path());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("resolves to  ", theme::dim()),
            Span::styled(resolved.display().to_string(), theme::muted()),
        ])),
        rows[1],
    );
    if let Some(error) = &config.export_error {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(error.clone(), theme::error()))),
            rows[2],
        );
    } else if config.export_confirm {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "The destination exists. Export again to confirm the overwrite.",
                theme::warn(),
            ))),
            rows[2],
        );
    } else if let Some(status) = &config.export_status {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(status.clone(), theme::accent()))),
            rows[2],
        );
    }
    // What travelled, without a value: fields, never a key.
    if let Some(document) = &config.exported {
        let lines = credential_summary(document);
        frame.render_widget(Paragraph::new(lines), rows[3]);
    }
    let hint = Line::from(Span::styled(
        "Enter export · Esc cancel · Tab switch",
        theme::muted(),
    ));
    frame.render_widget(Paragraph::new(hint), rows[11]);
}

fn draw_import(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let config = app.config();
    // Editor, status, two warnings, summary, hint: seven rows, the editor taking
    // whatever is left.
    let rows = rows_at(area, &[1, 1, 1, 1, 1, 1, 1]);
    let editor = rows[0];
    let text = if config.import.text.is_empty() {
        "Paste a portable configuration document (schema v1).\nCtrl+Enter validates and imports."
            .to_string()
    } else {
        config.import.text.clone()
    };
    let style = if config.import.text.is_empty() {
        theme::muted()
    } else {
        theme::text()
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(style)
            .wrap(Wrap { trim: false })
            .scroll((config.import.scroll as u16, 0)),
        editor,
    );
    let (line, column) = config.import.caret_position();
    let (label, style) = match (&config.plan, &config.import.error) {
        (Some(_), _) => ("valid".to_string(), theme::accent()),
        (None, Some(err)) => (err.message.clone(), theme::error()),
        (None, None) => ("not validated".to_string(), theme::muted()),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{line}:{column}  "), theme::dim()),
            Span::styled(label, style),
        ])),
        rows[1],
    );
    for (index, warning) in config.import.warnings.iter().take(2).enumerate() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(warning.clone(), theme::warn()))),
            rows[2 + index],
        );
    }
    if let Some(status) = &config.import_status {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(status.clone(), theme::accent()))),
            rows[4],
        );
    } else if config.plan.is_some() {
        // The redacted change summary the user commits against.
        let snapshot = ConfigurationSnapshot::load().unwrap_or_default();
        let changes = config.changes(&snapshot);
        let header = if changes.is_empty() {
            "nothing changes".to_string()
        } else {
            format!("{} change(s) to commit", changes.len())
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(header, theme::accent()))),
            rows[4],
        );
        for change in changes.iter().take(2) {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("{} {}: {}", change.area, change.id, change.summary),
                    theme::text(),
                ))),
                rows[5],
            );
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter newline · Ctrl+Enter validate+import · Esc cancel · Tab switch",
            theme::muted(),
        ))),
        rows[6],
    );
}

/// One row.
fn strip(area: ratatui::layout::Rect, top: u16, height: u16) -> ratatui::layout::Rect {
    ratatui::layout::Rect {
        x: area.x,
        y: area.y + top,
        width: area.width,
        height: height.min(area.height.saturating_sub(top)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_resolves_a_tilde_path() {
        assert_eq!(
            expand_home("~/argos-config.json"),
            paths::home_dir().join("argos-config.json")
        );
        assert_eq!(expand_home("/tmp/x.json"), PathBuf::from("/tmp/x.json"));
    }

    #[test]
    fn an_oversize_document_is_refused_before_anything_is_applied() {
        let mut config = ConfigView {
            tab: ConfigTab::Import,
            ..Default::default()
        };
        config.import.text = "x".repeat(config_transfer::MAX_DOCUMENT_BYTES + 1);
        config.validate();
        assert!(
            config.plan.is_none(),
            "an oversize document never arms Import"
        );
        assert!(config.import.error.is_some());
        let message = config.import.error.unwrap().message;
        assert!(message.contains("larger"), "{message}");
    }

    #[test]
    fn a_directory_target_is_an_actionable_error_not_a_crash() {
        let mut config = ConfigView {
            tab: ConfigTab::Export,
            ..Default::default()
        };
        config.export_path = "/".to_string();
        config.check_export_path();
        assert!(config.export_error.is_some());
        assert!(!config.export_confirm);
    }

    #[test]
    fn editing_the_buffer_disarms_the_import_button() {
        let mut config = ConfigView {
            tab: ConfigTab::Import,
            ..Default::default()
        };
        config.insert("{}");
        config.insert("\n");
        assert_eq!(config.import.caret, 3);
        config.backspace();
        assert_eq!(config.import.text, "{}");
        config.delete();
        assert_eq!(config.import.text, "{}");
        config.home();
        assert_eq!(config.import.caret_position().1, 1);
        config.end();
        assert_eq!(config.import.caret_position().1, 3);
        config.newline();
        assert_eq!(config.import.text, "{}\n");
    }

    #[test]
    fn a_validation_error_reports_a_pointer_and_never_a_value() {
        let message =
            "the configuration document is not valid: /providers/0/credential/api_key: a blank key is not a masked placeholder";
        let error = locate("{}", message);
        assert_eq!(error.pointer, "/providers/0/credential/api_key");
        assert!(!error.message.contains("sk-"));
    }

    #[test]
    fn the_pasted_secret_buffer_is_cleared_on_close() {
        let mut config = ConfigView::default();
        config.import.paste = "secret".to_string();
        config.close();
        assert!(
            config.import.paste.is_empty(),
            "no secret survives the popup"
        );
    }
}
