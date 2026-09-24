//! Session shell. The bottom prompt always talks to the view that is open:
//! the desk, the selected case, or the module on the canvas.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use anyhow::Result;
use argos_osint_core::agent::{self, HistMsg, TurnEvent, TurnInput};
use argos_osint_core::brain::Memory;
use argos_osint_core::gmail::{self, GmailConfig};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths::{self, db_label};
use argos_osint_core::prompt::{self, Intent};
use argos_osint_core::provider::{self, Poll, SettingsFile};
use argos_osint_core::report::{self, ReportMeta};
use argos_osint_core::search::SearchHit;
use argos_osint_core::secrets::{self, AuthFile, DeviceEndpoints, GmailSecret, ProviderSecret};
use argos_osint_core::session::{self, Case};
use argos_osint_core::store::{ChatLine, Store};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;
use sysinfo::System;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModuleId {
    Cases,
    Hardware,
    Providers,
    Brain,
    Gmail,
    Reports,
    Log,
    Settings,
}

impl ModuleId {
    pub fn title(self) -> &'static str {
        match self {
            Self::Cases => "Case Desk",
            Self::Hardware => "Hardware",
            Self::Providers => "Providers",
            Self::Brain => "Brain",
            Self::Gmail => "Gmail",
            Self::Reports => "Reports",
            Self::Log => "Search Log",
            Self::Settings => "Settings",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Cases => "Investigation chat and markdown reports",
            Self::Hardware => "Cores, RAM, VRAM, architecture",
            Self::Providers => "Text and voice model login",
            Self::Brain => "Facts recalled into the agent loop",
            Self::Gmail => "Gmail IMAP app password and MCP",
            Self::Reports => "Markdown reports on disk",
            Self::Log => "Search and tool stream",
            Self::Settings => "Layout, search endpoint, report folder",
        }
    }

    pub fn all() -> [ModuleId; 8] {
        [
            Self::Cases,
            Self::Hardware,
            Self::Providers,
            Self::Brain,
            Self::Gmail,
            Self::Reports,
            Self::Log,
            Self::Settings,
        ]
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim().to_lowercase();
        Self::all().into_iter().find(|m| {
            let title = m.title().to_lowercase();
            title == n || title.starts_with(&n) || format!("{:?}", m).to_lowercase() == n
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutMode {
    Classic,
    Dashboard,
    Tabs,
    Modal,
    Vertical,
    Horizontal,
    Three,
    Float,
    Grid,
    Zen,
}

impl LayoutMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Dashboard => "dashboard",
            Self::Tabs => "tabs",
            Self::Modal => "modal",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Three => "three",
            Self::Float => "float",
            Self::Grid => "grid",
            Self::Zen => "zen",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic Sidebar + Canvas",
            Self::Dashboard => "Two-Column Dashboard",
            Self::Tabs => "Top Tabs + Split",
            Self::Modal => "Focused Modal",
            Self::Vertical => "Vertical Split",
            Self::Horizontal => "Horizontal Split",
            Self::Three => "Three-Panel",
            Self::Float => "Floating Side Panel",
            Self::Grid => "Grid of Widgets",
            Self::Zen => "Minimal / Zen",
        }
    }

    pub fn all() -> [LayoutMode; 10] {
        [
            Self::Classic,
            Self::Dashboard,
            Self::Tabs,
            Self::Modal,
            Self::Vertical,
            Self::Horizontal,
            Self::Three,
            Self::Float,
            Self::Grid,
            Self::Zen,
        ]
    }

    pub fn parse(name: &str) -> Option<Self> {
        let n = name.trim().to_lowercase();
        Self::all()
            .into_iter()
            .find(|m| m.name() == n || m.label().to_lowercase().starts_with(&n))
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|m| *m == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Launcher,
    Canvas,
    Prompt,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Debug)]
pub enum AppMsg {
    Turn(TurnEvent),
    Hardware(HardwareProfile),
    Note(String),
    Search(Result<Vec<SearchHit>, String>),
    DeviceStatus(String),
    DeviceToken(String),
    Models(Result<Vec<String>, String>),
    Voice(Result<String, String>),
    GmailTest(Result<String, String>),
}

pub struct App {
    pub tx: UnboundedSender<AppMsg>,
    pub store: Store,
    pub settings: SettingsFile,
    pub auth: AuthFile,
    pub focus: Focus,
    pub layout: LayoutMode,
    pub launcher_sel: usize,
    pub module: Option<ModuleId>,
    pub tab_sel: usize,
    pub modal: bool,
    pub modal_query: String,
    pub modal_sel: usize,
    pub help: bool,
    pub prompt: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub hist_pos: Option<usize>,
    pub transcripts: HashMap<String, Vec<ChatLine>>,
    pub cases: Vec<Case>,
    pub case_sel: usize,
    pub memories: Vec<Memory>,
    pub brain_sel: usize,
    pub reports: Vec<ReportMeta>,
    pub log: Vec<String>,
    pub hardware: HardwareProfile,
    pub cpu_now: f32,
    pub cpu_hist: VecDeque<u64>,
    pub sys: System,
    pub fields: Vec<Field>,
    pub field_sel: usize,
    pub editing: bool,
    pub provider_slot: &'static str,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub spinner: usize,
    pub tick_n: u64,
    pub status: String,
    pub db_label: String,
    pub scroll_back: usize,
    pub quit: bool,
    pub launcher_area: Rect,
    pub quick_sel: usize,
}

impl App {
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        let store = Store::open(&paths::db_path())?;
        store.ensure_session("desk", "Desk", "desk")?;
        for module in ModuleId::all() {
            if module != ModuleId::Cases {
                store.ensure_session(&module_session(module), module.title(), "module")?;
            }
        }
        let settings = SettingsFile::load().unwrap_or_default();
        let auth = AuthFile::load().unwrap_or_default();
        let mut app = Self::from_parts(store, settings, auth)?;
        app.reload_lists()?;
        Ok(app)
    }

    pub fn from_parts(store: Store, settings: SettingsFile, auth: AuthFile) -> Result<Self> {
        let (tx, _rx) = unbounded_channel();
        let layout = LayoutMode::parse(&settings.layout).unwrap_or(LayoutMode::Classic);
        let mut app = Self {
            tx,
            store,
            settings,
            auth,
            focus: Focus::Prompt,
            layout,
            launcher_sel: 0,
            module: None,
            tab_sel: 0,
            modal: false,
            modal_query: String::new(),
            modal_sel: 0,
            help: false,
            prompt: String::new(),
            cursor: 0,
            history: Vec::new(),
            hist_pos: None,
            transcripts: HashMap::new(),
            cases: Vec::new(),
            case_sel: 0,
            memories: Vec::new(),
            brain_sel: 0,
            reports: Vec::new(),
            log: Vec::new(),
            hardware: HardwareProfile::unknown(),
            cpu_now: 0.0,
            cpu_hist: VecDeque::new(),
            sys: System::new(),
            fields: Vec::new(),
            field_sel: 0,
            editing: false,
            provider_slot: "text",
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            spinner: 0,
            tick_n: 0,
            status: "ready".into(),
            db_label: db_label(),
            scroll_back: 0,
            quit: false,
            launcher_area: Rect::default(),
            quick_sel: 0,
        };
        app.reload_lists()?;
        app.load_transcript(&app.session_id());
        Ok(app)
    }

    pub fn take_inbox(&mut self) -> UnboundedReceiver<AppMsg> {
        let (tx, rx) = unbounded_channel();
        self.tx = tx;
        rx
    }

    pub fn reload_lists(&mut self) -> Result<()> {
        self.cases = self.store.list_cases()?;
        self.memories = self.store.list_memories()?;
        self.reports = self.store.list_reports()?;
        if self.case_sel >= self.cases.len() && !self.cases.is_empty() {
            self.case_sel = 0;
        }
        Ok(())
    }

    pub fn session_id(&self) -> String {
        match self.module {
            None => "desk".into(),
            Some(ModuleId::Cases) => self
                .cases
                .get(self.case_sel)
                .map(|c| c.id.clone())
                .unwrap_or_else(|| "desk".into()),
            Some(module) => module_session(module),
        }
    }

    pub fn view_name(&self) -> String {
        match self.module {
            None => "Dashboard".into(),
            Some(ModuleId::Cases) => self
                .cases
                .get(self.case_sel)
                .map(|c| format!("Case · {}", c.title))
                .unwrap_or_else(|| "Case Desk".into()),
            Some(module) => module.title().to_string(),
        }
    }

    pub fn mode_label(&self) -> String {
        let run = if self.running {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            format!(" {} {}", frames[self.spinner % frames.len()], self.status)
        } else {
            String::new()
        };
        format!(
            "{} · {} · {}{run}",
            self.view_name(),
            self.settings.modality,
            self.layout.name(),
        )
    }

    pub fn transcript(&self) -> &[ChatLine] {
        self.transcripts
            .get(&self.session_id())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn load_transcript(&mut self, id: &str) {
        if self.transcripts.contains_key(id) {
            return;
        }
        let lines = self.store.load_messages(id).unwrap_or_default();
        self.transcripts.insert(id.to_string(), lines);
    }

    fn push_line(&mut self, role: &str, body: &str) {
        let id = self.session_id();
        let _ = self.store.append_message(&id, role, body);
        let line = ChatLine {
            role: role.into(),
            body: body.into(),
            created_at: String::new(),
        };
        self.transcripts.entry(id).or_default().push(line);
        self.scroll_back = 0;
    }

    fn replace_last_assistant(&mut self, body: &str) {
        let id = self.session_id();
        if let Some(lines) = self.transcripts.get_mut(&id) {
            if let Some(last) = lines.last_mut() {
                if last.role == "assistant" {
                    last.body = body.to_string();
                    return;
                }
            }
        }
        self.push_line("assistant", body);
    }

    pub fn view_context(&self) -> String {
        match self.module {
            None => "The dashboard launcher is open. Help the user pick an app or start a case. The prompt on this screen talks to the desk session.".into(),
            Some(ModuleId::Cases) => {
                if let Some(case) = self.cases.get(self.case_sel) {
                    format!("Case {} — {}. The user is looking at this investigation.", case.id, case.title)
                } else {
                    "No case is open. Suggest /new <title>.".into()
                }
            }
            Some(ModuleId::Hardware) => format!("Hardware profile is on screen.\n{}", self.hardware.one_line()),
            Some(ModuleId::Providers) => format!(
                "Provider setup is open ({} slot). Text: {}. Voice: {}.",
                self.provider_slot,
                provider_label(self.auth.text.as_ref()),
                provider_label(self.auth.voice.as_ref())
            ),
            Some(ModuleId::Brain) => format!("{} memories are stored. The user is looking at the brain.", self.memories.len()),
            Some(ModuleId::Gmail) => {
                if let Some(g) = &self.auth.gmail {
                    format!("Gmail setup is open for {}.", g.email)
                } else {
                    "Gmail is not connected. IMAP host is imap.gmail.com only.".into()
                }
            }
            Some(ModuleId::Reports) => format!("{} reports on disk.", self.reports.len()),
            Some(ModuleId::Log) => "The search and tool log is the main view.".into(),
            Some(ModuleId::Settings) => format!(
                "Settings. SearXNG: {}. Reports: {}.",
                if self.settings.searx_url.is_empty() { "public fallback" } else { self.settings.searx_url.as_str() },
                report_dir(&self.settings).display()
            ),
        }
    }

    pub fn filtered_modules(&self) -> Vec<ModuleId> {
        let q = self.modal_query.trim().to_lowercase();
        ModuleId::all()
            .into_iter()
            .filter(|m| {
                q.is_empty()
                    || m.title().to_lowercase().contains(&q)
                    || m.blurb().to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        self.tick_n = self.tick_n.wrapping_add(1);
        if self.tick_n % 5 != 0 {
            return;
        }
        self.sys.refresh_memory();
        self.sys.refresh_cpu_usage();
        let cpu = self.sys.global_cpu_usage();
        self.cpu_now = cpu;
        self.cpu_hist
            .push_back(cpu.round().clamp(0.0, 100.0) as u64);
        if self.cpu_hist.len() > 48 {
            self.cpu_hist.pop_front();
        }
        let total = self.sys.total_memory() as f64 / 1_073_741_824.0;
        let avail = self.sys.available_memory() as f64 / 1_073_741_824.0;
        if total > 0.0 {
            self.hardware.total_ram_gb = (total * 10.0).round() / 10.0;
            self.hardware.available_ram_gb = (avail * 10.0).round() / 10.0;
            self.hardware.cpu_usage = cpu;
            if self.hardware.logical_cores == 0 {
                self.hardware.logical_cores = self.sys.cpus().len();
            }
        }
    }

    pub fn spawn_hardware(&self, fresh: bool) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let profile =
                tokio::task::spawn_blocking(move || hardware::profile_cached(fresh)).await;
            if let Ok(profile) = profile {
                let _ = tx.send(AppMsg::Hardware(profile));
            }
        });
    }

    pub fn on_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::Turn(ev) => self.on_turn(ev),
            AppMsg::Hardware(profile) => {
                let cpu = self.cpu_now;
                let hist_cores = self.hardware.logical_cores;
                self.hardware = profile;
                if self.hardware.cpu_usage == 0.0 {
                    self.hardware.cpu_usage = cpu;
                }
                if self.hardware.logical_cores == 0 {
                    self.hardware.logical_cores = hist_cores;
                }
                self.log_note("hardware profile refreshed");
            }
            AppMsg::Note(text) => self.log_note(&text),
            AppMsg::Search(result) => self.finish_search(result),
            AppMsg::DeviceStatus(text) => {
                self.status = "device login".into();
                self.push_line("assistant", &text);
                self.log_note(&text);
            }
            AppMsg::DeviceToken(token) => {
                self.save_device_token(token);
            }
            AppMsg::Models(result) => match result {
                Ok(names) => {
                    let shown = if names.is_empty() {
                        "endpoint answered, no model ids".to_string()
                    } else {
                        format!(
                            "{} models, first: {}",
                            names.len(),
                            names.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
                        )
                    };
                    self.status = "provider ok".into();
                    self.push_line("assistant", &shown);
                }
                Err(err) => {
                    self.status = "provider error".into();
                    self.push_line("assistant", &err);
                }
            },
            AppMsg::Voice(result) => match result {
                Ok(text) => {
                    self.prompt = text;
                    self.cursor = self.prompt.chars().count();
                    self.status = "transcript ready".into();
                    self.focus = Focus::Prompt;
                }
                Err(err) => {
                    self.status = "voice error".into();
                    self.log_note(&err);
                    self.push_line("assistant", &err);
                }
            },
            AppMsg::GmailTest(result) => match result {
                Ok(text) => {
                    self.status = "gmail ok".into();
                    self.push_line("assistant", &text);
                }
                Err(err) => {
                    self.status = "gmail error".into();
                    self.push_line("assistant", &err);
                }
            },
        }
    }

    fn on_turn(&mut self, ev: TurnEvent) {
        match ev {
            TurnEvent::Status(text) => self.status = text,
            TurnEvent::Delta(text) => {
                let id = self.session_id();
                let lines = self.transcripts.entry(id).or_default();
                if let Some(last) = lines.last_mut() {
                    if last.role == "assistant" {
                        last.body.push_str(&text);
                    }
                }
            }
            TurnEvent::Note(text) => self.log_note(&text),
            TurnEvent::Report(meta) => {
                let _ = self.store.add_report(&meta);
                self.log_note(&format!("report {}", meta.path));
                let _ = self.reload_lists();
            }
            TurnEvent::Memory(memory) => {
                if let Ok(saved) = self.store.add_memory(&memory.text) {
                    self.memories.insert(0, saved);
                    self.log_note("brain updated");
                }
            }
            TurnEvent::Done(text) => {
                self.running = false;
                self.status = "ready".into();
                self.replace_last_assistant(&text);
            }
            TurnEvent::Failed(err) => {
                self.running = false;
                self.status = "error".into();
                self.replace_last_assistant(&err);
                self.log_note(&err);
            }
        }
    }

    fn finish_search(&mut self, result: Result<Vec<SearchHit>, String>) {
        self.running = false;
        match result {
            Ok(hits) => {
                let question = self.log.last().cloned().unwrap_or_else(|| "search".into());
                let title = question
                    .trim_start_matches("search ")
                    .trim()
                    .chars()
                    .take(72)
                    .collect::<String>();
                let md = report::source_pack(
                    if title.is_empty() { "Search" } else { &title },
                    &question,
                    &hits,
                );
                match report::write_report(
                    &report_dir(&self.settings),
                    &title,
                    self.case_id().as_deref(),
                    &md,
                ) {
                    Ok(meta) => {
                        let _ = self.store.add_report(&meta);
                        let _ = self.reload_lists();
                        self.push_line(
                            "assistant",
                            &format!("{}\n\nReport: {}", summarize_hits(&hits), meta.path),
                        );
                        self.log_note(&format!("{} hits", hits.len()));
                    }
                    Err(err) => self.push_line("assistant", &err.to_string()),
                }
            }
            Err(err) => self.push_line("assistant", &err),
        }
        self.status = "ready".into();
    }

    fn case_id(&self) -> Option<String> {
        if self.module == Some(ModuleId::Cases) {
            self.cases.get(self.case_sel).map(|c| c.id.clone())
        } else {
            None
        }
    }

    pub fn on_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Key(key) => self.on_key(key),
            Event::Mouse(mouse) => {
                if mouse.kind == MouseEventKind::Down(crossterm::event::MouseButton::Left) {
                    self.click(mouse.column, mouse.row);
                }
                false
            }
            _ => false,
        }
    }

    fn click(&mut self, x: u16, y: u16) {
        if self
            .launcher_area
            .contains(ratatui::layout::Position { x, y })
            && self.layout_has_launcher()
        {
            let rel = y.saturating_sub(self.launcher_area.y + 1) as usize;
            if rel < ModuleId::all().len() {
                self.launcher_sel = rel;
                self.open_module(ModuleId::all()[rel]);
            }
        }
    }

    fn layout_has_launcher(&self) -> bool {
        matches!(
            self.layout,
            LayoutMode::Classic
                | LayoutMode::Dashboard
                | LayoutMode::Vertical
                | LayoutMode::Three
                | LayoutMode::Modal
        )
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.help {
            self.help = false;
            return false;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            return self.cancel_or_quit();
        }
        if self.modal || self.layout == LayoutMode::Modal && self.focus != Focus::Prompt {
            return self.on_modal_key(key);
        }
        if ctrl && key.code == KeyCode::Char('p') {
            self.modal = true;
            self.modal_query.clear();
            self.modal_sel = 0;
            return false;
        }
        if ctrl && key.code == KeyCode::Char('l') {
            self.layout = self.layout.next();
            self.settings.layout = self.layout.name().into();
            let _ = self.settings.save();
            return false;
        }
        if ctrl && key.code == KeyCode::Char('r') && self.focus == Focus::Prompt {
            self.record_voice();
            return false;
        }
        if self.editing {
            return self.on_field_key(key);
        }
        match key.code {
            KeyCode::Tab => {
                self.focus = self.next_focus();
                false
            }
            KeyCode::Esc => {
                self.on_esc();
                false
            }
            KeyCode::Char('?') if self.focus != Focus::Prompt && self.prompt.is_empty() => {
                self.help = true;
                false
            }
            _ if self.focus == Focus::Prompt => self.on_prompt_key(key),
            _ if self.focus == Focus::Launcher => self.on_launcher_key(key),
            _ => self.on_canvas_key(key),
        }
    }

    fn cancel_or_quit(&mut self) -> bool {
        if self.running {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = "cancelling".into();
            false
        } else if !self.prompt.is_empty() && self.focus == Focus::Prompt {
            self.prompt.clear();
            self.cursor = 0;
            false
        } else {
            self.quit = true;
            true
        }
    }

    fn next_focus(&self) -> Focus {
        let order = if self.layout_has_launcher() {
            [Focus::Launcher, Focus::Canvas, Focus::Prompt]
        } else {
            [Focus::Canvas, Focus::Prompt, Focus::Prompt]
        };
        match self.focus {
            Focus::Launcher => Focus::Canvas,
            Focus::Canvas => Focus::Prompt,
            Focus::Prompt => order[0],
        }
    }

    fn on_esc(&mut self) {
        if self.modal {
            self.modal = false;
            return;
        }
        if self.editing {
            self.editing = false;
            return;
        }
        if self.module.is_some() {
            self.module = None;
            self.fields.clear();
            self.focus = if self.layout_has_launcher() {
                Focus::Launcher
            } else {
                Focus::Prompt
            };
            self.load_transcript("desk");
            self.scroll_back = 0;
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('p')) {
            self.modal = false;
            if self.layout == LayoutMode::Modal {
                self.layout = LayoutMode::Classic;
            }
            return false;
        }
        let n = self.filtered_modules().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if n > 0 {
                    self.modal_sel = (self.modal_sel + n - 1) % n;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if n > 0 {
                    self.modal_sel = (self.modal_sel + 1) % n;
                }
            }
            KeyCode::Enter => {
                if let Some(module) = self.filtered_modules().get(self.modal_sel).copied() {
                    self.modal = false;
                    self.open_module(module);
                }
            }
            KeyCode::Backspace => {
                self.modal_query.pop();
                self.modal_sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                self.modal_query.push(c);
                self.modal_sel = 0;
            }
            _ => {}
        }
        false
    }

    fn on_launcher_key(&mut self, key: KeyEvent) -> bool {
        let n = ModuleId::all().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.launcher_sel = (self.launcher_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.launcher_sel = (self.launcher_sel + 1) % n,
            KeyCode::Enter => self.open_module(ModuleId::all()[self.launcher_sel]),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let i = (c as usize) - ('1' as usize);
                if i < n {
                    self.launcher_sel = i;
                    self.open_module(ModuleId::all()[i]);
                }
            }
            _ => {}
        }
        false
    }

    fn on_canvas_key(&mut self, key: KeyEvent) -> bool {
        if !self.fields.is_empty()
            && matches!(
                self.module,
                Some(ModuleId::Providers | ModuleId::Gmail | ModuleId::Settings)
            )
        {
            let n = self.fields.len();
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.field_sel = (self.field_sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => self.field_sel = (self.field_sel + 1) % n,
                KeyCode::Enter => self.activate_field(),
                KeyCode::Char('r') if self.module == Some(ModuleId::Hardware) => {
                    self.spawn_hardware(true)
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_back = self.scroll_back.saturating_add(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_back = self.scroll_back.saturating_sub(1)
            }
            KeyCode::Char('r') if self.module == Some(ModuleId::Hardware) => {
                self.spawn_hardware(true)
            }
            _ => {}
        }
        if self.module == Some(ModuleId::Cases) && !self.cases.is_empty() {
            let n = self.cases.len();
            match key.code {
                KeyCode::Char('K') => {
                    self.case_sel = (self.case_sel + n - 1) % n;
                    self.bind_case();
                }
                KeyCode::Char('J') => {
                    self.case_sel = (self.case_sel + 1) % n;
                    self.bind_case();
                }
                _ => {}
            }
        }
        if self.module == Some(ModuleId::Brain) && !self.memories.is_empty() {
            let n = self.memories.len();
            match key.code {
                KeyCode::Char('x') | KeyCode::Delete => {
                    if let Some(mem) = self.memories.get(self.brain_sel) {
                        let id = mem.id.clone();
                        let _ = self.store.delete_memory(&id);
                        let _ = self.reload_lists();
                    }
                }
                KeyCode::Up => self.brain_sel = (self.brain_sel + n - 1) % n,
                KeyCode::Down => self.brain_sel = (self.brain_sel + 1) % n,
                _ => {}
            }
        }
        false
    }

    fn bind_case(&mut self) {
        if let Some(case) = self.cases.get(self.case_sel) {
            let id = case.id.clone();
            self.load_transcript(&id);
        }
        self.scroll_back = 0;
    }

    fn on_prompt_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let menu = self.prompt.starts_with('/') && !self.prompt.contains(' ');
        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let mut chars: Vec<char> = self.prompt.chars().collect();
                    chars.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.prompt = chars.into_iter().collect();
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.prompt.chars().count()),
            KeyCode::Up if menu => self.cycle_slash(-1),
            KeyCode::Down if menu => self.cycle_slash(1),
            KeyCode::Up => self.hist(-1),
            KeyCode::Down => self.hist(1),
            KeyCode::Char('u') if ctrl => {
                self.prompt.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                let mut chars: Vec<char> = self.prompt.chars().collect();
                let at = self.cursor.min(chars.len());
                chars.insert(at, c);
                self.cursor = at + 1;
                self.prompt = chars.into_iter().collect();
                self.hist_pos = None;
            }
            _ => {}
        }
        false
    }

    fn cycle_slash(&mut self, delta: isize) {
        let mut card = session::slash_menu(&self.prompt);
        card.selected = card
            .options
            .iter()
            .position(|o| self.prompt.trim_start_matches('/') == o.id)
            .unwrap_or(0);
        card.move_sel(delta);
        if let Some(opt) = card.selected() {
            self.prompt = opt.label.clone();
            self.cursor = self.prompt.chars().count();
        }
    }

    fn hist(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len() as isize;
        let cur = self.hist_pos.map(|p| p as isize).unwrap_or(len);
        let next = (cur + delta).clamp(0, len);
        if next == len {
            self.hist_pos = None;
            return;
        }
        self.hist_pos = Some(next as usize);
        self.prompt = self.history[next as usize].clone();
        self.cursor = self.prompt.chars().count();
    }

    fn submit(&mut self) {
        let line = self.prompt.trim().to_string();
        if line.is_empty() {
            return;
        }
        self.history.push(line.clone());
        self.hist_pos = None;
        self.prompt.clear();
        self.cursor = 0;
        if line.starts_with('/') {
            self.run_slash(&line);
        } else {
            self.spawn_turn(line);
        }
    }

    fn run_slash(&mut self, line: &str) {
        let rest = line.trim_start_matches('/').trim();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "help" | "?" => self.help = true,
            "quit" | "exit" | "q" => self.quit = true,
            "dashboard" | "home" => self.on_esc(),
            "layout" => {
                if let Some(mode) = LayoutMode::parse(&arg) {
                    self.layout = mode;
                    self.settings.layout = mode.name().into();
                    let _ = self.settings.save();
                } else {
                    self.push_line("assistant", "Layouts: classic, dashboard, tabs, modal, vertical, horizontal, three, float, grid, zen");
                }
            }
            "new" => {
                let title = if arg.is_empty() {
                    "Untitled case".into()
                } else {
                    arg
                };
                if let Ok(case) = self.store.create_case(&title) {
                    let _ = self.reload_lists();
                    self.case_sel = self.cases.iter().position(|c| c.id == case.id).unwrap_or(0);
                    self.open_module(ModuleId::Cases);
                    self.push_line("assistant", &format!("Opened {} ({})", case.title, case.id));
                }
            }
            "use" => match session::resolve_case(&self.cases, &arg) {
                Some(case) => {
                    self.case_sel = self.cases.iter().position(|c| c.id == case.id).unwrap_or(0);
                    self.open_module(ModuleId::Cases);
                    self.bind_case();
                }
                None => self.push_line("assistant", "No single case matches that query."),
            },
            "search" => {
                if arg.is_empty() {
                    self.push_line("assistant", "Usage: /search <query>");
                } else {
                    self.spawn_search(arg);
                }
            }
            "report" => self.write_visible_report(&arg),
            "hardware" => {
                self.open_module(ModuleId::Hardware);
                self.spawn_hardware(arg == "fresh");
            }
            "provider" | "login" => self.open_module(ModuleId::Providers),
            "brain" => {
                if arg.is_empty() {
                    self.open_module(ModuleId::Brain);
                } else if let Ok(mem) = self.store.add_memory(&arg) {
                    self.memories.insert(0, mem);
                    self.push_line("assistant", &format!("Remembered: {arg}"));
                }
            }
            "gmail" => self.open_module(ModuleId::Gmail),
            "voice" => {
                self.settings.modality = "voice".into();
                let _ = self.settings.save();
                self.push_line(
                    "assistant",
                    "Modality is voice. Ctrl+R records, Enter sends the transcript.",
                );
            }
            "text" => {
                self.settings.modality = "text".into();
                let _ = self.settings.save();
                self.push_line("assistant", "Modality is text.");
            }
            "open" => {
                if let Some(module) = ModuleId::from_name(&arg) {
                    self.open_module(module);
                } else {
                    self.push_line("assistant", "Unknown app. Ctrl+P lists them.");
                }
            }
            "clear" => {
                let id = self.session_id();
                let _ = self.store.clear_messages(&id);
                self.transcripts.insert(id, Vec::new());
            }
            other => self.push_line(
                "assistant",
                &format!("Unknown command /{other}. /help lists them."),
            ),
        }
    }

    fn write_visible_report(&mut self, title: &str) {
        let title = if title.is_empty() {
            self.view_name()
        } else {
            title.to_string()
        };
        let mut body = String::new();
        for line in self.transcript() {
            body.push_str(&format!("**{}:** {}\n\n", line.role, line.body));
        }
        let md = report::render_report(&title, self.case_id().as_deref(), "", &body, &[]);
        match report::write_report(
            &report_dir(&self.settings),
            &title,
            self.case_id().as_deref(),
            &md,
        ) {
            Ok(meta) => {
                let _ = self.store.add_report(&meta);
                let _ = self.reload_lists();
                self.push_line("assistant", &format!("Report: {}", meta.path));
            }
            Err(err) => self.push_line("assistant", &err.to_string()),
        }
    }

    fn spawn_search(&mut self, query: String) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        self.running = true;
        self.status = "searching".into();
        self.push_line("user", &format!("/search {query}"));
        self.push_line("assistant", "");
        self.log_note(&format!("search {query}"));
        let searx = self.settings.searx_url.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = argos_osint_core::search::web_search(
                &query,
                Some(searx.as_str()).filter(|s| !s.is_empty()),
            )
            .await;
            let _ = tx.send(AppMsg::Search(result));
        });
    }

    fn spawn_turn(&mut self, text: String) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        if prompt::classify(&text) == Intent::Remember {
            let fact = prompt::remember_text(&text);
            self.push_line("user", &text);
            if let Ok(mem) = self.store.add_memory(&fact) {
                self.memories.insert(0, mem);
                self.push_line("assistant", &format!("Remembered: {fact}"));
            }
            return;
        }
        self.running = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = "starting".into();
        self.push_line("user", &text);
        self.push_line("assistant", "");
        let id = self.session_id();
        let history = self
            .transcript()
            .iter()
            .rev()
            .skip(2)
            .take(16)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter(|l| l.role == "user" || l.role == "assistant")
            .map(|l| HistMsg {
                role: l.role.clone(),
                content: l.body.clone(),
            })
            .collect();
        let input = TurnInput {
            session_id: id,
            user_text: text,
            history,
            memories: self.memories.clone(),
            view_name: self.view_name(),
            view_context: self.view_context(),
            hardware_line: self.hardware.one_line(),
            modality: self.settings.modality.clone(),
            provider: self.auth.text.clone(),
            searx_url: Some(self.settings.searx_url.clone()).filter(|s| !s.is_empty()),
            report_dir: report_dir(&self.settings),
            case_id: self.case_id(),
            gmail: self.auth.gmail.as_ref().map(GmailConfig::from),
        };
        let tx = self.tx.clone();
        let cancel = Arc::clone(&self.cancel);
        tokio::spawn(async move {
            let (atx, mut arx) = unbounded_channel();
            let worker = tokio::spawn(async move {
                agent::run_turn(input, atx, cancel).await;
            });
            while let Some(ev) = arx.recv().await {
                if tx.send(AppMsg::Turn(ev)).is_err() {
                    break;
                }
            }
            let _ = worker.await;
        });
    }

    pub fn open_module(&mut self, module: ModuleId) {
        self.module = Some(module);
        self.tab_sel = ModuleId::all()
            .iter()
            .position(|m| *m == module)
            .unwrap_or(0);
        self.focus = Focus::Prompt;
        self.scroll_back = 0;
        self.editing = false;
        self.load_fields(module);
        if module == ModuleId::Cases {
            self.bind_case();
        } else {
            let id = module_session(module);
            self.load_transcript(&id);
        }
        if module == ModuleId::Hardware {
            self.spawn_hardware(false);
        }
    }

    fn load_fields(&mut self, module: ModuleId) {
        self.fields.clear();
        self.field_sel = 0;
        match module {
            ModuleId::Providers => {
                let secret = self.slot_secret();
                self.fields = vec![
                    field(
                        "kind",
                        "Kind (local / api / device)",
                        secret
                            .as_ref()
                            .map(|s| s.kind.clone())
                            .unwrap_or_else(|| "local".into()),
                        false,
                    ),
                    field(
                        "base_url",
                        "Base URL",
                        secret
                            .as_ref()
                            .map(|s| s.base_url.clone())
                            .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into()),
                        false,
                    ),
                    field(
                        "model",
                        "Model",
                        secret
                            .as_ref()
                            .map(|s| s.model.clone())
                            .unwrap_or_else(|| "llama3.2".into()),
                        false,
                    ),
                    field(
                        "api_key",
                        "API key",
                        secret
                            .as_ref()
                            .and_then(|s| s.api_key.clone())
                            .unwrap_or_default(),
                        true,
                    ),
                    field(
                        "stt_model",
                        "Voice model",
                        secret
                            .as_ref()
                            .and_then(|s| s.stt_model.clone())
                            .unwrap_or_else(|| "whisper-1".into()),
                        false,
                    ),
                    field(
                        "client_id",
                        "Device client id",
                        secret
                            .as_ref()
                            .and_then(|s| s.device.as_ref().map(|d| d.client_id.clone()))
                            .unwrap_or_default(),
                        false,
                    ),
                    field(
                        "device_auth_url",
                        "Device authorization URL",
                        secret
                            .as_ref()
                            .and_then(|s| s.device.as_ref().map(|d| d.device_auth_url.clone()))
                            .unwrap_or_default(),
                        false,
                    ),
                    field(
                        "token_url",
                        "Token URL",
                        secret
                            .as_ref()
                            .and_then(|s| s.device.as_ref().map(|d| d.token_url.clone()))
                            .unwrap_or_default(),
                        false,
                    ),
                    field(
                        "scope",
                        "Scope",
                        secret
                            .as_ref()
                            .and_then(|s| s.device.as_ref().map(|d| d.scope.clone()))
                            .unwrap_or_else(|| "openid profile email".into()),
                        false,
                    ),
                    field(
                        "__slot",
                        "Slot action: press enter to flip text/voice",
                        self.provider_slot.into(),
                        false,
                    ),
                    field("__save", "Save this slot", "enter".into(), false),
                    field("__test", "Test /models", "enter".into(), false),
                    field("__device", "Start device-code login", "enter".into(), false),
                ];
            }
            ModuleId::Gmail => {
                let g = self.auth.gmail.clone();
                self.fields = vec![
                    field(
                        "email",
                        "Gmail address",
                        g.as_ref().map(|g| g.email.clone()).unwrap_or_default(),
                        false,
                    ),
                    field(
                        "app_password",
                        "App password",
                        g.as_ref()
                            .map(|g| g.app_password.clone())
                            .unwrap_or_default(),
                        true,
                    ),
                    field("__save", "Save Gmail", "enter".into(), false),
                    field("__test", "Test INBOX", "enter".into(), false),
                    field("__mcp", "Write MCP config", "enter".into(), false),
                ];
            }
            ModuleId::Settings => {
                self.fields = vec![
                    field(
                        "searx_url",
                        "SearXNG base URL (empty uses DuckDuckGo)",
                        self.settings.searx_url.clone(),
                        false,
                    ),
                    field(
                        "report_dir",
                        "Report directory (empty uses ./reports)",
                        self.settings.report_dir.clone(),
                        false,
                    ),
                    field("layout", "Layout name", self.layout.name().into(), false),
                    field("__save", "Save settings", "enter".into(), false),
                ];
            }
            _ => {}
        }
    }

    fn slot_secret(&self) -> Option<ProviderSecret> {
        if self.provider_slot == "voice" {
            self.auth.voice.clone()
        } else {
            self.auth.text.clone()
        }
    }

    fn field_value(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    fn activate_field(&mut self) {
        let key = self
            .fields
            .get(self.field_sel)
            .map(|f| f.key.clone())
            .unwrap_or_default();
        if key.starts_with("__") {
            self.run_field_action(&key);
        } else {
            self.editing = true;
        }
    }

    fn run_field_action(&mut self, key: &str) {
        match (self.module, key) {
            (Some(ModuleId::Providers), "__slot") => {
                self.provider_slot = if self.provider_slot == "text" {
                    "voice"
                } else {
                    "text"
                };
                self.load_fields(ModuleId::Providers);
            }
            (Some(ModuleId::Providers), "__save") => self.save_provider_fields(),
            (Some(ModuleId::Providers), "__test") => self.test_provider(),
            (Some(ModuleId::Providers), "__device") => self.start_device(),
            (Some(ModuleId::Gmail), "__save") => self.save_gmail_fields(),
            (Some(ModuleId::Gmail), "__test") => self.test_gmail(),
            (Some(ModuleId::Gmail), "__mcp") => self.write_mcp(),
            (Some(ModuleId::Settings), "__save") => self.save_settings_fields(),
            _ => {}
        }
    }

    fn on_field_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.push(c);
                }
            }
            _ => {}
        }
        false
    }

    fn save_provider_fields(&mut self) {
        let secret = ProviderSecret {
            kind: empty_fallback(&self.field_value("kind"), "local"),
            base_url: self.field_value("base_url"),
            model: self.field_value("model"),
            api_key: Some(self.field_value("api_key")).filter(|s| !s.is_empty()),
            stt_model: Some(self.field_value("stt_model")).filter(|s| !s.is_empty()),
            device: Some(DeviceEndpoints {
                client_id: self.field_value("client_id"),
                device_auth_url: self.field_value("device_auth_url"),
                token_url: self.field_value("token_url"),
                scope: empty_fallback(&self.field_value("scope"), "openid profile email"),
            })
            .filter(|d| !d.client_id.is_empty()),
        };
        if self.provider_slot == "voice" {
            self.auth.voice = Some(secret);
        } else {
            self.auth.text = Some(secret);
        }
        match self.auth.save() {
            Ok(()) => self.push_line(
                "assistant",
                &format!("Saved the {} provider.", self.provider_slot),
            ),
            Err(err) => self.push_line("assistant", &err.to_string()),
        }
    }

    fn save_device_token(&mut self, token: String) {
        let mut secret = self.slot_secret().unwrap_or(ProviderSecret {
            kind: "device".into(),
            base_url: self.field_value("base_url"),
            model: self.field_value("model"),
            api_key: None,
            stt_model: None,
            device: None,
        });
        secret.kind = "device".into();
        secret.api_key = Some(token);
        if self.provider_slot == "voice" {
            self.auth.voice = Some(secret);
        } else {
            self.auth.text = Some(secret);
        }
        let _ = self.auth.save();
        self.load_fields(ModuleId::Providers);
        self.push_line(
            "assistant",
            "Device login stored a token. The token is not shown.",
        );
        self.status = "ready".into();
    }

    fn test_provider(&mut self) {
        self.save_provider_fields();
        let Some(secret) = self.slot_secret() else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = provider::list_models(&secret)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::Models(result));
        });
        self.status = "contacting provider".into();
    }

    fn start_device(&mut self) {
        self.save_provider_fields();
        let Some(secret) = self.slot_secret() else {
            return;
        };
        let Some(endpoints) = secret.device.clone() else {
            self.push_line(
                "assistant",
                "Fill in the device client id, authorization URL, and token URL first.",
            );
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match provider::start_device(&endpoints).await {
                Err(err) => {
                    let _ = tx.send(AppMsg::DeviceStatus(err.to_string()));
                }
                Ok(grant) => {
                    let show = format!(
                        "Open {} and enter code {}.\n{}",
                        grant.verification_uri,
                        grant.user_code,
                        grant.verification_uri_complete.unwrap_or_default()
                    );
                    let _ = tx.send(AppMsg::DeviceStatus(show));
                    let mut interval = grant.interval.max(2);
                    let mut waited = 0u64;
                    loop {
                        if waited >= grant.expires_in {
                            let _ = tx.send(AppMsg::DeviceStatus("Device code expired.".into()));
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(interval)).await;
                        waited = waited.saturating_add(interval);
                        match provider::poll_device(&endpoints, &grant.device_code).await {
                            Ok(Poll::Pending) => {}
                            Ok(Poll::SlowDown) => interval = interval.saturating_add(5),
                            Ok(Poll::Token(token)) => {
                                let _ = tx.send(AppMsg::DeviceToken(token));
                                break;
                            }
                            Ok(Poll::Denied(err)) => {
                                let _ = tx.send(AppMsg::DeviceStatus(err));
                                break;
                            }
                            Err(err) => {
                                let _ = tx.send(AppMsg::DeviceStatus(err.to_string()));
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    fn save_gmail_fields(&mut self) {
        let secret = GmailSecret {
            email: self.field_value("email").trim().to_string(),
            app_password: self.field_value("app_password"),
        };
        let cfg = GmailConfig::from(&secret);
        if let Err(err) = gmail::validate(&cfg) {
            self.push_line("assistant", &err);
            return;
        }
        self.auth.gmail = Some(secret);
        match self.auth.save() {
            Ok(()) => self.push_line(
                "assistant",
                "Saved Gmail. The app password stays in ~/.argos/auth.json.",
            ),
            Err(err) => self.push_line("assistant", &err.to_string()),
        }
    }

    fn test_gmail(&mut self) {
        self.save_gmail_fields();
        let Some(cfg) = self.auth.gmail.as_ref().map(GmailConfig::from) else {
            return;
        };
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result =
                gmail::inbox_count(&cfg).map(|n| format!("INBOX is reachable. {n} messages."));
            let _ = tx.send(AppMsg::GmailTest(result));
        });
        self.status = "checking gmail".into();
    }

    fn write_mcp(&mut self) {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "argos".into());
        let body = gmail::mcp_config_json(&exe);
        let path = paths::mcp_path();
        match secrets::write_private(
            &path,
            &serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::json!({})),
            )
            .unwrap_or(body),
        ) {
            Ok(()) => self.push_line(
                "assistant",
                &format!("Wrote {}. Launch with `argos mcp gmail`.", path.display()),
            ),
            Err(err) => self.push_line("assistant", &err.to_string()),
        }
    }

    fn save_settings_fields(&mut self) {
        self.settings.searx_url = self.field_value("searx_url");
        self.settings.report_dir = self.field_value("report_dir");
        if let Some(mode) = LayoutMode::parse(&self.field_value("layout")) {
            self.layout = mode;
            self.settings.layout = mode.name().into();
        }
        match self.settings.save() {
            Ok(()) => self.push_line("assistant", "Settings saved."),
            Err(err) => self.push_line("assistant", &err.to_string()),
        }
    }

    fn record_voice(&mut self) {
        let Some(secret) = self.auth.voice.clone().or_else(|| self.auth.text.clone()) else {
            self.push_line(
                "assistant",
                "Set a voice provider first. The endpoint must implement /audio/transcriptions.",
            );
            return;
        };
        self.status = "listening".into();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match record_wav().await {
                Err(err) => {
                    let _ = tx.send(AppMsg::Voice(Err(err)));
                }
                Ok(bytes) => {
                    let _ = tx.send(AppMsg::Note(format!("captured {} bytes", bytes.len())));
                    match provider::transcribe(&secret, &bytes).await {
                        Ok(text) => {
                            let _ = tx.send(AppMsg::Voice(Ok(text)));
                        }
                        Err(err) => {
                            let _ = tx.send(AppMsg::Voice(Err(err.to_string())));
                        }
                    }
                }
            }
        });
    }

    fn log_note(&mut self, text: &str) {
        for line in text.lines() {
            if !line.trim().is_empty() {
                self.log.push(line.trim().to_string());
            }
        }
        if self.log.len() > 200 {
            let drain = self.log.len() - 200;
            self.log.drain(0..drain);
        }
    }
}

fn module_session(module: ModuleId) -> String {
    format!("module:{}", module.title().to_lowercase().replace(' ', "-"))
}

fn field(key: &str, label: &str, value: String, secret: bool) -> Field {
    Field {
        key: key.into(),
        label: label.into(),
        value,
        secret,
    }
}

fn empty_fallback(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value.trim().into()
    }
}

pub fn provider_label(secret: Option<&ProviderSecret>) -> String {
    match secret {
        Some(secret) if !secret.model.is_empty() => format!("{} {}", secret.kind, secret.model),
        _ => "not signed in".into(),
    }
}

pub fn report_dir(settings: &SettingsFile) -> PathBuf {
    if !settings.report_dir.trim().is_empty() {
        return PathBuf::from(settings.report_dir.trim());
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("reports")
}

fn summarize_hits(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No public hits.".into();
    }
    let mut out = String::new();
    for (i, hit) in hits.iter().take(8).enumerate() {
        out.push_str(&format!("{}. {} — {}\n", i + 1, hit.title, hit.url));
    }
    out
}

async fn record_wav() -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!("argos-voice-{}.wav", std::process::id()));
    let path_str = path.display().to_string();
    let attempts = [
        vec![
            "rec", "-q", "-r", "16000", "-c", "1", &path_str, "trim", "0", "5",
        ],
        vec![
            "ffmpeg",
            "-y",
            "-f",
            "avfoundation",
            "-i",
            ":0",
            "-t",
            "5",
            "-ac",
            "1",
            "-ar",
            "16000",
            &path_str,
        ],
    ];
    let mut last = "no recorder found".to_string();
    for args in attempts {
        let mut cmd = tokio::process::Command::new(args[0]);
        cmd.args(&args[1..])
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null());
        match cmd.output().await {
            Ok(out) if out.status.success() && path.exists() => {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&path);
                if bytes.len() < 64 {
                    return Err("recording was empty".into());
                }
                return Ok(bytes);
            }
            Ok(out) => last = format!("{}: {}", args[0], String::from_utf8_lossy(&out.stderr)),
            Err(err) => last = format!("{}: {err}", args[0]),
        }
    }
    let _ = std::fs::remove_file(&path);
    Err(format!("Voice capture needs sox (`rec`) or ffmpeg. {last}"))
}

pub async fn run(mut app: App) -> Result<()> {
    use crossterm::event::EventStream;
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use futures_util::StreamExt;
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    let mut inbox = app.take_inbox();
    app.spawn_hardware(false);
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(
        out,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let mut reader = EventStream::new();
    let _guard = RawRestorer;
    loop {
        terminal.draw(|frame| super::ui::draw(frame, &mut app))?;
        if app.quit {
            break;
        }
        tokio::select! {
            biased;
            msg = inbox.recv() => {
                if let Some(msg) = msg {
                    app.on_msg(msg);
                }
            }
            ev = reader.next() => {
                match ev {
                    Some(Ok(ev)) => { app.on_event(ev); }
                    Some(Err(_)) | None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => app.tick(),
        }
        if app.quit {
            break;
        }
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    let _ = terminal.show_cursor();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_osint_core::provider::SettingsFile;
    use argos_osint_core::secrets::AuthFile;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn classic_frame_mentions_the_launcher_and_prompt() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.hardware.cpu_name = "Test CPU".into();
        app.hardware.logical_cores = 8;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("APPLICATION LAUNCHER"), "{text}");
        assert!(text.contains("Ctrl+P"), "{text}");
        assert_eq!(super::super::slash_menu("/use").option_count(), 1);
    }
}

struct RawRestorer;
impl Drop for RawRestorer {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
    }
}
