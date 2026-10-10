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

use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use argos_osint_core::config_transfer::{
    self, ConfigurationSnapshot, ImportPlan, ProfileConfig, QuotaSettingsFile, SCHEMA_VERSION,
};
use argos_osint_core::paths;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::secrets::AuthFile;

use super::app::App;
use super::theme;

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
    /// Redacted changes computed by Verify, never by the renderer.
    pub change_summary: Vec<String>,
    validated_text: Option<String>,
}

impl Default for ConfigView {
    fn default() -> Self {
        Self {
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
            change_summary: Vec::new(),
            validated_text: None,
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

    /// Enter inserts a newline; applying requires explicit button activation.
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

    pub fn move_vertical(&mut self, delta: isize) {
        let (line, column) = self.caret_position();
        let lines: Vec<_> = self.text.split('\n').collect();
        let next = (line - 1)
            .saturating_add_signed(delta)
            .min(lines.len().saturating_sub(1));
        self.caret = lines
            .iter()
            .take(next)
            .map(|line| line.chars().count() + 1)
            .sum::<usize>()
            + (column - 1).min(lines[next].chars().count());
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
        self.validated_text = None;
        self.change_summary.clear();
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
        self.import.text.clear();
        self.import.caret = 0;
        self.disarm();
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
        let confirmed = self
            .export_confirm
            .then(|| self.export_resolved.clone())
            .flatten();
        self.check_export_path();
        if self.export_error.is_some() {
            anyhow::bail!("the destination is not writable");
        }
        if self.export_confirm && confirmed != self.export_resolved {
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
                match ConfigurationSnapshot::load() {
                    Ok(snapshot) => {
                        self.change_summary = self
                            .changes(&snapshot)
                            .iter()
                            .map(|change| {
                                format!("{} {}: {}", change.area, change.id, change.summary)
                            })
                            .collect();
                        self.validated_text = Some(self.import.text.clone());
                    }
                    Err(_) => {
                        self.plan = None;
                        self.import_error =
                            Some("Could not load the current configuration baseline".into());
                    }
                }
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
        if self.validated_text.as_deref() != Some(self.import.text.as_str()) {
            self.disarm();
            anyhow::bail!("verify this editor revision first");
        }
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
    let tokens: Vec<_> = message.split_whitespace().collect();
    for pair in tokens.windows(2) {
        let value = pair[1]
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok();
        match (pair[0], value) {
            ("line", Some(value)) => line = value,
            ("column", Some(value)) => column = value,
            _ => {}
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

/// The warning the popup shows on the credential export path.
pub const SECRET_WARNING: &str =
    "This file contains saved API keys. Treat it as a secret and never commit it.";

/// Configs page: Export precedes the independent multiline Import editor.
/// Rendering consumes the validation summary prepared by Verify.
pub fn draw(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    use super::app::{FieldId, Target};
    use super::components;
    use ratatui::layout::Rect;
    let config = app.config();
    let block = theme::card(" Configs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 10 {
        frame.render_widget(
            Paragraph::new("Configs needs more height · resize to edit").style(theme::warn()),
            inner,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(SECRET_WARNING).style(theme::warn()),
        strip(inner, 0, 1),
    );
    let button_width = 12.min(inner.width);
    let field = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width.saturating_sub(button_width + 1),
        3,
    );
    let export = Rect::new(field.right() + 1, field.y, button_width, 3);
    app.draw_export_field(frame, field);
    app.layout
        .borrow_mut()
        .register(Target::ConfigAction(0), export);
    components::tab_button(
        frame,
        export,
        "Export",
        false,
        app.focus == Target::ConfigAction(0),
    );
    let status = config
        .export_error
        .as_ref()
        .or(config.export_status.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if config.export_confirm {
                "Destination exists · Export again to overwrite".into()
            } else {
                config
                    .export_resolved
                    .as_ref()
                    .map_or_else(|| config.export_path.clone(), |p| p.display().to_string())
            }
        });
    frame.render_widget(
        Paragraph::new(status).style(theme::muted()),
        strip(inner, 4, 1),
    );
    let editor_height = inner.height.saturating_sub(11).max(3);
    let editor = strip(inner, 5, editor_height);
    let focused = app.focus == Target::Field(FieldId::ProfileImportEditor);
    let block = super::ui::focused_pane(" Import JSON ", focused);
    let viewport = block.inner(editor);
    frame.render_widget(block, editor);
    app.layout
        .borrow_mut()
        .register(Target::Field(FieldId::ProfileImportEditor), editor);
    let (line, column) = config.import.caret_position();
    let offset = if focused {
        config
            .import
            .scroll
            .min(line.saturating_sub(1))
            .max(line.saturating_sub(viewport.height as usize))
    } else {
        config.import.scroll
    };
    let text: Vec<Line<'static>> = if config.import.text.is_empty() {
        vec![Line::styled("Paste schema v1 JSON here", theme::muted())]
    } else {
        config
            .import
            .text
            .split('\n')
            .skip(offset)
            .take(viewport.height as usize)
            .map(|line| Line::raw(components::clip_text(line, viewport.width as usize)))
            .collect()
    };
    frame.render_widget(Paragraph::new(text), viewport);
    if focused && line > offset && line - offset <= viewport.height as usize && viewport.width > 0 {
        let current_line = config.import.text.split('\n').nth(line - 1).unwrap_or("");
        let before: String = current_line.chars().take(column - 1).collect();
        frame.set_cursor_position((
            viewport.x + (components::text_width(&before) as u16).min(viewport.width - 1),
            viewport.y + (line - offset - 1) as u16,
        ));
    }
    let y = 5 + editor_height;
    let buttons = strip(inner, y, 1);
    let verify = Rect::new(buttons.x, buttons.y, 10.min(buttons.width), 1);
    let save = Rect::new(
        verify.right(),
        buttons.y,
        buttons.width.saturating_sub(verify.width).min(24),
        1,
    );
    for (action, rect, label) in [(1, verify, "Verify"), (2, save, "Save and apply")] {
        if action == 1 || config.plan.is_some() {
            app.layout
                .borrow_mut()
                .register(Target::ConfigAction(action), rect);
        }
        components::tab_button(
            frame,
            rect,
            label,
            false,
            app.focus == Target::ConfigAction(action),
        );
    }
    let validation = config
        .import_error
        .as_ref()
        .or(config.import_status.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if config.plan.is_some() {
                format!(
                    "Verified · {} redacted changes",
                    config.change_summary.len()
                )
            } else {
                "Save disabled · verify this revision first".into()
            }
        });
    frame.render_widget(
        Paragraph::new(validation).style(theme::muted()),
        strip(inner, y + 1, 1),
    );
    let summary: Vec<Line<'static>> = config
        .import
        .warnings
        .iter()
        .chain(config.change_summary.iter())
        .map(|line| Line::raw(line.clone()))
        .collect();
    frame.render_widget(
        Paragraph::new(summary).wrap(Wrap { trim: false }),
        strip(inner, y + 2, inner.height.saturating_sub(y + 3)),
    );
    frame.render_widget(
        Paragraph::new("Tab focus · Enter newline in JSON · Verify → Save and apply · Esc discard")
            .style(theme::dim()),
        strip(inner, inner.height - 1, 1),
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
    #[test]
    fn syntax_error_location_uses_actual_line_and_column() {
        let error = locate("", "expected value at line 7 column 12");
        assert_eq!((error.line, error.column), (7, 12));
    }
    #[test]
    fn confirmed_export_overwrites_the_same_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "old").unwrap();
        let mut view = ConfigView {
            export_path: path.display().to_string(),
            ..Default::default()
        };
        assert!(view.run_export().is_err());
        assert!(view.export_confirm);
        assert!(view.run_export().is_ok());
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("schema_version"));
    }
    #[test]
    fn direct_buffer_change_invalidates_commit_revision() {
        let mut view = ConfigView::default();
        view.import.text = "edited without Verify".into();
        assert!(view.run_import().is_err());
        assert!(view.plan.is_none());
        assert_eq!(view.import.text, "edited without Verify");
    }
}
