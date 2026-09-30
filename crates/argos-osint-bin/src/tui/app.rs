//! App state and keyboard routing for the Argos terminal shell.

use anyhow::Result;
use argos_osint_core::brain::{Memory, MemorySource, ScoredMemory};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::Terminal;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleId {
    Brain,
    Providers,
    System,
}
impl ModuleId {
    pub const ALL: [Self; 3] = [Self::Brain, Self::Providers, Self::System];
    pub fn title(self) -> &'static str {
        match self {
            Self::Brain => "Brain",
            Self::Providers => "Providers",
            Self::System => "System",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Self::Brain => "Recall insights from conversations",
            Self::Providers => "Grok, OpenAI, OpenRouter, Models",
            Self::System => "Hardware and settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderPage {
    Grok,
    OpenAI,
    OpenRouter,
    Models,
}
impl ProviderPage {
    pub const ALL: [Self; 4] = [Self::Grok, Self::OpenAI, Self::OpenRouter, Self::Models];
    pub fn title(self) -> &'static str {
        match self {
            Self::Grok => "Grok",
            Self::OpenAI => "OpenAI",
            Self::OpenRouter => "OpenRouter",
            Self::Models => "Models",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldId {
    BrainApp,
    BrainConversation,
    BrainInsight,
    BrainQuery,
    WriterProvider,
    WriterModel,
    RouterKey,
    RouterEndpoint,
    Composer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonId {
    Add,
    Recall,
    Pin,
    Delete,
    SaveWriter,
    GrokSignIn,
    GrokCheck,
    OpenAISignIn,
    OpenAICheck,
    RouterSave,
    RouterVerify,
    RouterAdvanced,
    RefreshHardware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    App(usize),
    ProviderTab(ProviderPage),
    Memory(usize),
    Field(FieldId),
    Button(ButtonId),
}

#[derive(Debug)]
enum ProviderEvent {
    Progress {
        page: ProviderPage,
        line: String,
    },
    Finished {
        page: ProviderPage,
        result: Result<String, String>,
    },
}

pub struct App {
    pub module: Option<ModuleId>,
    pub launcher_sel: usize,
    pub provider_page: ProviderPage,
    pub input: String,
    pub brain_app: String,
    pub brain_conversation: String,
    pub brain_insight: String,
    pub brain_query: String,
    pub writer_provider: String,
    pub writer_model: String,
    pub router_key: String,
    pub router_endpoint: String,
    pub router_advanced: bool,
    pub grok_status: String,
    pub openai_status: String,
    pub router_status: String,
    pub provider_progress: Vec<String>,
    pub provider_progress_page: Option<ProviderPage>,
    pub provider_pending: Option<ProviderPage>,
    pub focus: Target,
    pub cursor: usize,
    pub screen: Rect,
    pub status: String,
    pub memories: Vec<Memory>,
    pub memory_sel: usize,
    pub hits: Vec<ScoredMemory>,
    pub auth: AuthFile,
    pub settings: SettingsFile,
    pub hardware: HardwareProfile,
    auth_path: PathBuf,
    store: Store,
    provider_tx: UnboundedSender<ProviderEvent>,
    provider_rx: UnboundedReceiver<ProviderEvent>,
}

impl App {
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        let store = Store::open(&paths::db_path())?;
        let memories = store.list_memories()?;
        let auth = AuthFile::load()?;
        let settings = SettingsFile::load()?;
        let router = provider::account_secret(&auth, "openrouter");
        let (provider_tx, provider_rx) = unbounded_channel();
        Ok(Self {
            module: None,
            launcher_sel: 0,
            provider_page: ProviderPage::Grok,
            input: String::new(),
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            writer_provider: settings.writer_provider.clone(),
            writer_model: settings.writer_model.clone(),
            router_key: router.api_key.unwrap_or_default(),
            router_endpoint: router.base_url,
            router_advanced: false,
            grok_status: "Not checked · Sign in or check existing login".into(),
            openai_status: "Not checked · Sign in or check existing login".into(),
            router_status: "Enter a key, then verify or save".into(),
            provider_progress: Vec::new(),
            provider_progress_page: None,
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::default(),
            status: "ready".into(),
            memories,
            memory_sel: 0,
            hits: Vec::new(),
            auth,
            settings,
            hardware: hardware::profile_cached(false),
            auth_path: paths::auth_path(),
            store,
            provider_tx,
            provider_rx,
        })
    }

    fn select(&mut self, index: usize) {
        self.launcher_sel = index;
        self.module = Some(ModuleId::ALL[index]);
        self.hits.clear();
        self.status = format!("{} open", ModuleId::ALL[index].title());
        self.set_focus(match self.module {
            Some(ModuleId::Brain) => Target::Field(FieldId::BrainApp),
            Some(ModuleId::Providers) => Target::ProviderTab(self.provider_page),
            _ => Target::Button(ButtonId::RefreshHardware),
        });
    }

    pub fn field(&self, field: FieldId) -> &str {
        match field {
            FieldId::BrainApp => &self.brain_app,
            FieldId::BrainConversation => &self.brain_conversation,
            FieldId::BrainInsight => &self.brain_insight,
            FieldId::BrainQuery => &self.brain_query,
            FieldId::WriterProvider => &self.writer_provider,
            FieldId::WriterModel => &self.writer_model,
            FieldId::RouterKey => &self.router_key,
            FieldId::RouterEndpoint => &self.router_endpoint,
            FieldId::Composer => &self.input,
        }
    }

    fn field_mut(&mut self, field: FieldId) -> &mut String {
        match field {
            FieldId::BrainApp => &mut self.brain_app,
            FieldId::BrainConversation => &mut self.brain_conversation,
            FieldId::BrainInsight => &mut self.brain_insight,
            FieldId::BrainQuery => &mut self.brain_query,
            FieldId::WriterProvider => &mut self.writer_provider,
            FieldId::WriterModel => &mut self.writer_model,
            FieldId::RouterKey => &mut self.router_key,
            FieldId::RouterEndpoint => &mut self.router_endpoint,
            FieldId::Composer => &mut self.input,
        }
    }

    fn set_focus(&mut self, target: Target) {
        self.focus = target;
        self.cursor = match target {
            Target::Field(field) => self.field(field).chars().count(),
            _ => 0,
        };
    }

    fn save_insight(&mut self) -> Result<String> {
        let (category, text) = argos_osint_core::brain::parse_typed_memory(&self.brain_insight);
        self.store.add_memory(
            &text,
            category,
            false,
            MemorySource {
                app: self.brain_app.trim().into(),
                conversation_id: self.brain_conversation.trim().into(),
                message_id: None,
                reference: None,
            },
        )?;
        self.memories = self.store.list_memories()?;
        self.brain_insight.clear();
        Ok("Insight saved with source".into())
    }

    fn activate_button(&mut self, button: ButtonId) {
        let result = match button {
            ButtonId::Add => self.save_insight(),
            ButtonId::Recall => self.store.recall(&self.brain_query, 8).map(|hits| {
                self.hits = hits;
                format!("{} relevant memories", self.hits.len())
            }),
            ButtonId::Pin => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store
                    .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)
                    .and_then(|_| self.store.list_memories())
                    .map(|memories| {
                        self.memories = memories;
                        "Memory pin updated".into()
                    })
            }
            ButtonId::Delete => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store
                    .delete_memory(&memory.id)
                    .and_then(|_| self.store.list_memories())
                    .map(|memories| {
                        self.memories = memories;
                        self.memory_sel =
                            self.memory_sel.min(self.memories.len().saturating_sub(1));
                        "Memory deleted".into()
                    })
            }
            ButtonId::SaveWriter => {
                let provider = self.writer_provider.trim();
                let model = self.writer_model.trim();
                if provider.is_empty() || model.is_empty() {
                    Err(anyhow::anyhow!("Provider and model are required"))
                } else {
                    let kind = provider::normalize_kind(provider);
                    let kind = if kind == "openai" {
                        "openai-chatgpt".into()
                    } else {
                        kind
                    };
                    if !matches!(
                        kind.as_str(),
                        "grok" | "openai-chatgpt" | "openrouter" | "local"
                    ) {
                        self.status = "Choose Grok, OpenAI, OpenRouter, or local".into();
                        return;
                    }
                    self.settings.writer_provider = kind;
                    self.settings.writer_model = model.into();
                    self.settings
                        .save()
                        .map(|_| format!("Writer: {} / {}", self.settings.writer_provider, model))
                }
            }
            ButtonId::GrokSignIn => {
                self.start_subscription(ProviderPage::Grok, true);
                return;
            }
            ButtonId::GrokCheck => {
                self.start_subscription(ProviderPage::Grok, false);
                return;
            }
            ButtonId::OpenAISignIn => {
                self.start_subscription(ProviderPage::OpenAI, true);
                return;
            }
            ButtonId::OpenAICheck => {
                self.start_subscription(ProviderPage::OpenAI, false);
                return;
            }
            ButtonId::RouterSave => self.save_router(),
            ButtonId::RouterVerify => {
                self.verify_router();
                return;
            }
            ButtonId::RouterAdvanced => {
                self.router_advanced = !self.router_advanced;
                Ok(if self.router_advanced {
                    "Advanced endpoint shown"
                } else {
                    "Advanced endpoint hidden"
                }
                .into())
            }
            ButtonId::RefreshHardware => {
                self.hardware = hardware::profile_cached(true);
                Ok("Hardware refreshed".into())
            }
        };
        self.status = result.unwrap_or_else(|err| err.to_string());
    }

    fn router_draft(&self) -> Result<ProviderSecret> {
        let mut secret = provider::account_secret(&self.auth, "openrouter");
        secret.api_key = Some(self.router_key.trim().to_string()).filter(|key| !key.is_empty());
        secret.base_url = provider::normalize_base(self.router_endpoint.trim());
        let endpoint = url::Url::parse(&secret.base_url)
            .map_err(|_| anyhow::anyhow!("Enter a valid HTTPS API endpoint"))?;
        anyhow::ensure!(
            endpoint.scheme() == "https"
                && endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none(),
            "Use an HTTPS endpoint without credentials, query, or fragment"
        );
        Ok(secret)
    }

    fn save_router(&mut self) -> Result<String> {
        let secret = self.router_draft()?;
        let mut auth = self.auth.clone();
        auth.set_account(secret);
        auth.save_to(&self.auth_path)?;
        self.auth = auth;
        self.router_status = "OpenRouter account saved · verify to test access".into();
        Ok(self.router_status.clone())
    }

    fn verify_router(&mut self) {
        if self.provider_pending.is_some() {
            self.status = "Another provider check is running".into();
            return;
        }
        let secret = match self.router_draft() {
            Ok(secret) => secret,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        let saved = provider::account_secret(&self.auth, "openrouter");
        let draft = secret.api_key != saved.api_key || secret.base_url != saved.base_url;
        self.provider_pending = Some(ProviderPage::OpenRouter);
        self.router_status = "Verifying OpenRouter connection…".into();
        self.status = self.router_status.clone();
        let tx = self.provider_tx.clone();
        tokio::spawn(async move {
            let result = provider::verified_catalog(&secret)
                .await
                .map(|models| {
                    format!(
                        "OpenRouter verified · {} models{}",
                        models.len(),
                        if draft { " · Save to use" } else { "" }
                    )
                })
                .map_err(|err| err.to_string());
            let _ = tx.send(ProviderEvent::Finished {
                page: ProviderPage::OpenRouter,
                result,
            });
        });
    }

    fn start_subscription(&mut self, page: ProviderPage, login: bool) {
        if self.provider_pending.is_some() {
            self.status = "Another provider sign-in is running".into();
            return;
        }
        self.provider_pending = Some(page);
        self.provider_progress.clear();
        self.provider_progress_page = Some(page);
        let label = if page == ProviderPage::Grok {
            "Grok"
        } else {
            "ChatGPT"
        };
        let status = format!("{} {}…", if login { "Starting" } else { "Checking" }, label);
        if page == ProviderPage::Grok {
            self.grok_status = status.clone();
        } else {
            self.openai_status = status.clone();
        }
        self.status = status;
        let tx = self.provider_tx.clone();
        tokio::spawn(async move {
            let result = if page == ProviderPage::Grok {
                let outcome = if login {
                    let progress_tx = tx.clone();
                    argos_osint_core::grok_oauth::login(move |line| {
                        let _ = progress_tx.send(ProviderEvent::Progress {
                            page,
                            line: line.into(),
                        });
                    })
                    .await
                } else {
                    argos_osint_core::grok_oauth::check_login().await
                };
                match outcome {
                    Ok(_) => {
                        let secret = provider::account_secret(&AuthFile::default(), "grok");
                        provider::verified_catalog(&secret)
                            .await
                            .map(|models| {
                                format!("Grok subscription connected · {} models", models.len())
                            })
                            .map_err(|err| err.to_string())
                    }
                    Err(err) => Err(err.to_string()),
                }
            } else {
                let outcome = if login {
                    let progress_tx = tx.clone();
                    argos_osint_core::subscription::login(move |line| {
                        let _ = progress_tx.send(ProviderEvent::Progress {
                            page,
                            line: line.into(),
                        });
                    })
                    .await
                } else {
                    argos_osint_core::subscription::check_login().await
                };
                outcome.map_err(|err| err.to_string())
            };
            let _ = tx.send(ProviderEvent::Finished { page, result });
        });
    }

    fn on_provider_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Progress { page, line } => {
                if self.provider_pending == Some(page) {
                    self.provider_progress.push(line);
                    if self.provider_progress.len() > 6 {
                        self.provider_progress.remove(0);
                    }
                }
            }
            ProviderEvent::Finished { page, result } => {
                if self.provider_pending != Some(page) {
                    return;
                }
                self.provider_pending = None;
                let message = result.unwrap_or_else(|err| err);
                match page {
                    ProviderPage::Grok => self.grok_status = message.clone(),
                    ProviderPage::OpenAI => self.openai_status = message.clone(),
                    ProviderPage::OpenRouter => self.router_status = message.clone(),
                    ProviderPage::Models => {}
                }
                self.status = message;
            }
        }
    }

    fn activate_target(&mut self, target: Target) {
        match target {
            Target::App(index) => self.select(index),
            Target::ProviderTab(page) => {
                self.provider_page = page;
                self.set_focus(target);
            }
            Target::Memory(index) => {
                self.memory_sel = index;
                self.set_focus(target);
            }
            Target::Field(_) => self.set_focus(target),
            Target::Button(button) => {
                self.set_focus(target);
                self.activate_button(button);
            }
        }
    }

    fn edit_char(&mut self, character: char) {
        let Target::Field(field) = self.focus else {
            return;
        };
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let byte = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.insert(byte, character);
        self.cursor += 1;
    }

    fn edit_backspace(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if self.cursor == 0 {
            return;
        }
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let start = value
            .char_indices()
            .nth(cursor - 1)
            .map(|(byte, _)| byte)
            .unwrap_or(0);
        let end = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn edit_delete(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        let cursor = self.cursor;
        let value = self.field_mut(field);
        let start = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        let end = value
            .char_indices()
            .nth(cursor + 1)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.replace_range(start..end, "");
    }

    fn submit(&mut self) {
        let input = std::mem::take(&mut self.input);
        let input = input.trim();
        if input.is_empty() {
            return;
        }
        let result = match self.module {
            Some(ModuleId::Brain) => self.brain_command(input),
            Some(ModuleId::Providers) => self.provider_command(input),
            Some(ModuleId::System) if input == "refresh" => {
                self.hardware = hardware::profile_cached(true);
                Ok("Hardware refreshed".into())
            }
            _ => Err(anyhow::anyhow!(
                "Open Brain or Providers to use the composer"
            )),
        };
        self.status = match result {
            Ok(message) => message,
            Err(err) => format!("{err}"),
        };
    }

    fn brain_command(&mut self, input: &str) -> Result<String> {
        if let Some(query) = input.strip_prefix("recall ") {
            self.hits = self.store.recall(query, 8)?;
            return Ok(format!("{} relevant memories", self.hits.len()));
        }
        if let Some(rest) = input.strip_prefix("add ") {
            let (source, text) = rest
                .split_once('|')
                .ok_or_else(|| anyhow::anyhow!("Use: add <app> <conversation-id> | <insight>"))?;
            let mut parts = source.split_whitespace();
            let app = parts.next().unwrap_or_default();
            let conversation_id = parts.next().unwrap_or_default();
            anyhow::ensure!(
                parts.next().is_none(),
                "Use one app and one conversation ID"
            );
            let (category, text) = argos_osint_core::brain::parse_typed_memory(text);
            self.store.add_memory(
                &text,
                category,
                false,
                MemorySource {
                    app: app.into(),
                    conversation_id: conversation_id.into(),
                    message_id: None,
                    reference: None,
                },
            )?;
            self.memories = self.store.list_memories()?;
            return Ok("Insight saved with source".into());
        }
        if input == "pin" {
            let memory = self
                .memories
                .get(self.memory_sel)
                .ok_or_else(|| anyhow::anyhow!("No memory selected"))?;
            self.store
                .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)?;
            self.memories = self.store.list_memories()?;
            return Ok("Memory pin updated".into());
        }
        if input == "delete" {
            let memory = self
                .memories
                .get(self.memory_sel)
                .ok_or_else(|| anyhow::anyhow!("No memory selected"))?;
            self.store.delete_memory(&memory.id)?;
            self.memories = self.store.list_memories()?;
            self.memory_sel = self.memory_sel.min(self.memories.len().saturating_sub(1));
            return Ok("Memory deleted".into());
        }
        Err(anyhow::anyhow!("Use add, recall, pin, or delete"))
    }

    fn provider_command(&mut self, input: &str) -> Result<String> {
        if let Some(rest) = input.strip_prefix("writer ") {
            let mut parts = rest.split_whitespace();
            let provider = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Use: writer <provider> <model>"))?;
            let model = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("Use: writer <provider> <model>"))?;
            anyhow::ensure!(parts.next().is_none(), "Use one provider and one model ID");
            let kind = provider::normalize_kind(provider);
            let kind = if kind == "openai" {
                "openai-chatgpt".into()
            } else {
                kind
            };
            anyhow::ensure!(
                matches!(
                    kind.as_str(),
                    "grok" | "openai-chatgpt" | "openrouter" | "local"
                ),
                "Choose Grok, OpenAI, OpenRouter, or local"
            );
            self.settings.writer_provider = kind;
            self.settings.writer_model = model.into();
            self.settings.save()?;
            self.writer_provider = self.settings.writer_provider.clone();
            self.writer_model = self.settings.writer_model.clone();
            return Ok(format!(
                "Writer: {} / {}",
                self.settings.writer_provider, model
            ));
        }
        Err(anyhow::anyhow!(
            "Use: writer <provider> <model>; select a provider tab to sign in"
        ))
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
            if let Target::Field(field) = self.focus {
                self.field_mut(field).clear();
                self.cursor = 0;
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.input.clear();
                self.module = None;
                self.hits.clear();
                self.set_focus(Target::App(self.launcher_sel));
            }
            KeyCode::Enter => match self.focus {
                Target::Field(FieldId::Composer) => self.submit(),
                Target::Field(_) => self.focus_next(false),
                target => self.activate_target(target),
            },
            KeyCode::Backspace => self.edit_backspace(),
            KeyCode::Delete => self.edit_delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => {
                if let Target::Field(field) = self.focus {
                    self.cursor = (self.cursor + 1).min(self.field(field).chars().count());
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => {
                if let Target::Field(field) = self.focus {
                    self.cursor = self.field(field).chars().count();
                }
            }
            KeyCode::Char(c) => self.edit_char(c),
            KeyCode::Up if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = self.memory_sel.saturating_sub(1);
                self.set_focus(Target::Memory(self.memory_sel));
            }
            KeyCode::Down if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = (self.memory_sel + 1).min(self.memories.len().saturating_sub(1));
                self.set_focus(Target::Memory(self.memory_sel));
            }
            KeyCode::Up => self.launcher_sel = self.launcher_sel.saturating_sub(1),
            KeyCode::Down => {
                self.launcher_sel = (self.launcher_sel + 1).min(ModuleId::ALL.len() - 1)
            }
            KeyCode::Tab => self.focus_next(key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::F(n) if (1..=ModuleId::ALL.len() as u8).contains(&n) => {
                self.select(n as usize - 1)
            }
            _ => {}
        }
        true
    }

    fn focus_next(&mut self, reverse: bool) {
        let order = super::ui::focus_order(self);
        if order.is_empty() {
            return;
        }
        let current = order
            .iter()
            .position(|target| *target == self.focus)
            .unwrap_or(0);
        let next = if reverse {
            (current + order.len() - 1) % order.len()
        } else {
            (current + 1) % order.len()
        };
        self.set_focus(order[next]);
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(target) = super::ui::hit_test(self, mouse.column, mouse.row) {
                    self.activate_target(target);
                    if let Target::Field(field) = target {
                        self.cursor = super::ui::cursor_at(self, field, mouse.column);
                    }
                }
            }
            MouseEventKind::ScrollDown if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = (self.memory_sel + 1).min(self.memories.len().saturating_sub(1));
            }
            MouseEventKind::ScrollUp if self.module == Some(ModuleId::Brain) => {
                self.memory_sel = self.memory_sel.saturating_sub(1);
            }
            _ => {}
        }
    }
}

pub async fn run(mut app: App) -> Result<()> {
    use crossterm::{
        execute,
        terminal::{enable_raw_mode, EnterAlternateScreen},
    };
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen
            );
        }
    }
    enable_raw_mode()?;
    let _restore = Restore;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let mut terminal: Terminal<ratatui::backend::CrosstermBackend<Stdout>> =
        Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    loop {
        while let Ok(message) = app.provider_rx.try_recv() {
            app.on_provider_event(message);
        }
        terminal.draw(|frame| {
            app.screen = frame.area();
            super::ui::draw(frame, &app)
        })?;
        let next = if app.provider_pending.is_some() {
            if event::poll(Duration::from_millis(150))? {
                Some(event::read()?)
            } else {
                None
            }
        } else {
            Some(event::read()?)
        };
        match next {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press && !app.handle_key(key) => {
                break
            }
            Some(Event::Key(_)) => {}
            Some(Event::Mouse(mouse)) => app.handle_mouse(mouse),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (provider_tx, provider_rx) = unbounded_channel();
        App {
            module: None,
            launcher_sel: 0,
            provider_page: ProviderPage::Grok,
            input: String::new(),
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            writer_provider: String::new(),
            writer_model: String::new(),
            router_key: String::new(),
            router_endpoint: "https://openrouter.ai/api/v1".into(),
            router_advanced: false,
            grok_status: "Not checked".into(),
            openai_status: "Not checked".into(),
            router_status: "Not checked".into(),
            provider_progress: Vec::new(),
            provider_progress_page: None,
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::new(0, 0, 100, 34),
            status: String::new(),
            memories: Vec::new(),
            memory_sel: 0,
            hits: Vec::new(),
            auth: AuthFile::default(),
            settings: SettingsFile::default(),
            hardware: HardwareProfile::unknown(),
            auth_path: PathBuf::new(),
            store: Store::memory().unwrap(),
            provider_tx,
            provider_rx,
        }
    }

    fn click(app: &mut App, target: Target) {
        let position = (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .find(|(x, y)| super::super::ui::hit_test(app, *x, *y) == Some(target))
            .expect("visible click target");
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: position.0,
            row: position.1,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
    }

    #[test]
    fn brain_tab_still_edits_and_recalls_sourced_memories() {
        let mut app = app();
        click(&mut app, Target::App(0));
        click(&mut app, Target::Field(FieldId::BrainApp));
        type_text(&mut app, "chat");
        click(&mut app, Target::Field(FieldId::BrainConversation));
        type_text(&mut app, "thread-1");
        click(&mut app, Target::Field(FieldId::BrainInsight));
        type_text(&mut app, "project: Atlas launch");
        click(&mut app, Target::Button(ButtonId::Add));
        assert_eq!(app.memories[0].source.conversation_id, "thread-1");
        click(&mut app, Target::Field(FieldId::BrainQuery));
        type_text(&mut app, "Atlas");
        click(&mut app, Target::Button(ButtonId::Recall));
        assert_eq!(app.hits.len(), 1);
    }

    #[test]
    fn provider_auth_tabs_and_router_form_are_clickable() {
        let mut app = app();
        click(&mut app, Target::App(1));
        for page in ProviderPage::ALL {
            click(&mut app, Target::ProviderTab(page));
            assert_eq!(app.provider_page, page);
        }
        click(&mut app, Target::ProviderTab(ProviderPage::OpenRouter));
        click(&mut app, Target::Field(FieldId::RouterKey));
        type_text(&mut app, "secret");
        assert_eq!(app.router_key, "secret");
        click(&mut app, Target::Button(ButtonId::RouterAdvanced));
        assert!(app.router_advanced);
        click(&mut app, Target::Field(FieldId::RouterEndpoint));
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        type_text(&mut app, "invalid-");
        click(&mut app, Target::Button(ButtonId::RouterSave));
        assert!(app.status.contains("HTTPS"));
        assert!(app.auth.account("openrouter").is_none());
    }

    #[test]
    fn openrouter_save_keeps_other_accounts_and_writer_routing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.auth_path = dir.path().join("auth.json");
        let mut other = provider::account_secret(&app.auth, "local");
        other.api_key = Some("other-secret".into());
        app.auth.set_account(other);
        app.settings.writer_provider = "grok".into();
        click(&mut app, Target::App(1));
        click(&mut app, Target::ProviderTab(ProviderPage::OpenRouter));
        click(&mut app, Target::Field(FieldId::RouterKey));
        type_text(&mut app, "router-secret");
        click(&mut app, Target::Button(ButtonId::RouterSave));
        assert_eq!(
            app.auth.account("openrouter").unwrap().api_key.as_deref(),
            Some("router-secret")
        );
        assert_eq!(
            app.auth.account("local").unwrap().api_key.as_deref(),
            Some("other-secret")
        );
        assert_eq!(app.settings.writer_provider, "grok");
        let saved = std::fs::read_to_string(&app.auth_path).unwrap();
        assert!(saved.contains("router-secret"));
    }

    #[test]
    fn subscription_progress_and_result_update_the_correct_page() {
        let mut app = app();
        app.provider_pending = Some(ProviderPage::Grok);
        app.on_provider_event(ProviderEvent::Progress {
            page: ProviderPage::Grok,
            line: "Open browser".into(),
        });
        assert_eq!(app.provider_progress, ["Open browser"]);
        app.on_provider_event(ProviderEvent::Finished {
            page: ProviderPage::Grok,
            result: Ok("Grok subscription connected · 2 models".into()),
        });
        assert!(app.grok_status.contains("connected"));
        assert_eq!(app.openai_status, "Not checked");
        assert_eq!(app.provider_pending, None);
    }

    #[test]
    fn all_provider_actions_render_with_hit_areas_at_80x24() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        app.select(1);
        for (page, target) in [
            (ProviderPage::Grok, Target::Button(ButtonId::GrokSignIn)),
            (ProviderPage::OpenAI, Target::Button(ButtonId::OpenAISignIn)),
            (
                ProviderPage::OpenRouter,
                Target::Button(ButtonId::RouterVerify),
            ),
            (ProviderPage::Models, Target::Button(ButtonId::SaveWriter)),
        ] {
            app.provider_page = page;
            terminal
                .draw(|frame| super::super::ui::draw(frame, &app))
                .unwrap();
            assert!((0..24)
                .flat_map(|y| (0..80).map(move |x| (x, y)))
                .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)));
        }
    }
}
