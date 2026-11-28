//! Bounded article listing and discovery through the connected backend.

use std::collections::HashSet;

use chrono::NaiveDate;
use chrono::Utc;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::ipc::ArticlesSearchQuery;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::TextEdit;
use serde::Deserialize;
use serde::Serialize;

use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

const ARTICLES_PATH: &str = "/api/v1/articles";
const ARTICLES_SEARCH_PATH: &str = "/api/v1/articles/search";
const PAGE_SIZE: u16 = 20;
const MAX_PAGES: usize = 12;
const MAX_ROWS: usize = 200;
const MAX_QUERY_BYTES: usize = 256;
const MAX_CURSOR_BYTES: usize = 4096;
const DEBOUNCE_SECONDS: f64 = 0.2;

/// The article list displays only discovery metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ArticleRow {
    #[serde(alias = "article_id")]
    pub id: String,
    pub run_id: String,
    pub title: String,
    pub source_name: String,
    pub published_at: String,
    #[serde(default)]
    pub excerpt: Option<String>,
}

#[derive(Debug, Default)]
pub struct ArticleResults {
    pub rows: Vec<ArticleRow>,
    pub truncated: bool,
}

#[derive(Deserialize)]
struct ArticlePage {
    #[serde(alias = "hits")]
    rows: Vec<ArticleRow>,
    next_cursor: Option<String>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum ArticleScope {
    Browse(String),
    Search(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Authority {
    connection_epoch: u64,
    origin_revision: u64,
}

struct ArticleRequest {
    authority: Authority,
    revision: u64,
    scope: ArticleScope,
    cursor: Option<String>,
}

/// One active article query, with cursor pages fenced to its current origin.
pub struct ArticlesSearch {
    pub browse_day: TextEdit,
    pub status: Load<ArticleResults>,
    authority: Option<Authority>,
    scope: Option<ArticleScope>,
    revision: u64,
    pending_at: Option<f64>,
    in_flight: bool,
    request: Option<RequestTask>,
    next_cursor: Option<String>,
    seen_cursors: HashSet<String>,
    pages: usize,
    tail_error: Option<String>,
}

impl Default for ArticlesSearch {
    fn default() -> Self {
        Self {
            browse_day: TextEdit::new(Utc::now().date_naive().to_string()),
            status: Load::Idle,
            authority: None,
            scope: None,
            revision: 0,
            pending_at: None,
            in_flight: false,
            request: None,
            next_cursor: None,
            seen_cursors: HashSet::new(),
            pages: 0,
            tail_error: None,
        }
    }
}

impl ArticlesSearch {
    pub fn can_load_more(&self) -> bool {
        !self.in_flight && self.next_cursor.is_some() && matches!(self.status, Load::Ready(_))
    }

    pub fn tail_loading(&self) -> bool {
        self.in_flight && matches!(self.status, Load::Ready(_))
    }

    pub fn tail_error(&self) -> Option<&str> {
        self.tail_error.as_deref()
    }

    pub fn can_retry(&self) -> bool {
        self.scope.is_some() && matches!(self.status, Load::Failed(_))
    }

    /// The applied UTC run day can differ from the user's unsubmitted draft.
    pub fn applied_day(&self) -> Option<&str> {
        match &self.scope {
            Some(ArticleScope::Browse(day)) => Some(day),
            _ => None,
        }
    }

    /// Cancel a read and fence any completion already posted to the UI thread.
    pub(crate) fn cancel_pending(&mut self) {
        self.request = None;
        self.pending_at = None;
        self.in_flight = false;
        self.revision = self.revision.wrapping_add(1);
        if matches!(self.status, Load::Loading | Load::Pending) {
            self.status = Load::Idle;
        }
    }

    pub(crate) fn reset(&mut self) {
        self.change_scope(None);
        self.authority = None;
    }

    fn change_scope(&mut self, scope: Option<ArticleScope>) {
        self.cancel_pending();
        self.scope = scope;
        self.status = Load::Idle;
        self.next_cursor = None;
        self.seen_cursors.clear();
        self.pages = 0;
        self.tail_error = None;
    }

    fn start(&mut self, tail: bool) -> Option<ArticleRequest> {
        if self.in_flight {
            return None;
        }
        let authority = self.authority?;
        let scope = self.scope.clone()?;
        let cursor = if tail {
            Some(self.next_cursor.clone()?)
        } else {
            self.status = Load::Loading;
            None
        };
        self.pending_at = None;
        self.in_flight = true;
        self.tail_error = None;
        self.revision = self.revision.wrapping_add(1);
        Some(ArticleRequest {
            authority,
            revision: self.revision,
            scope,
            cursor,
        })
    }

    fn complete(&mut self, ticket: ArticleRequest, result: Result<ArticlePage, ClientError>) {
        if self.authority != Some(ticket.authority)
            || self.revision != ticket.revision
            || !self.in_flight
        {
            return;
        }
        self.request = None;
        self.in_flight = false;
        match result {
            Ok(page) => self.append(page),
            Err(error)
                if ticket.cursor.is_some()
                    && !matches!(error, ClientError::Status(401 | 403 | 409)) =>
            {
                self.tail_error = Some("More articles could not be loaded.".into());
            }
            Err(error) => {
                self.change_scope(Some(ticket.scope));
                self.status = Load::Failed(
                    match error {
                        ClientError::Status(401 | 403) => {
                            "The current connection cannot read articles."
                        }
                        ClientError::Status(400) => "The backend rejected this article request.",
                        ClientError::Status(409) => "The article request scope changed. Try again.",
                        _ => "Articles could not be loaded.",
                    }
                    .into(),
                );
            }
        }
    }

    fn append(&mut self, page: ArticlePage) {
        let mut results = match std::mem::replace(&mut self.status, Load::Idle) {
            Load::Ready(existing) => existing,
            _ => ArticleResults::default(),
        };
        let mut seen: HashSet<String> = results.rows.iter().map(|row| row.id.clone()).collect();
        let mut new_rows = page
            .rows
            .into_iter()
            .filter(|row| seen.insert(row.id.clone()));
        let available = MAX_ROWS.saturating_sub(results.rows.len());
        results.rows.extend(new_rows.by_ref().take(available));
        self.pages += 1;

        let can_continue = results.rows.len() < MAX_ROWS && self.pages < MAX_PAGES;
        self.next_cursor = page.next_cursor.as_ref().and_then(|cursor| {
            (can_continue && self.seen_cursors.insert(cursor.clone())).then(|| cursor.clone())
        });
        results.truncated |= page.truncated
            || new_rows.next().is_some()
            || (self.next_cursor.is_none() && page.next_cursor.is_some());
        self.status = Load::Ready(results);
    }
}

impl State {
    pub fn can_read_articles(&self) -> bool {
        self.permission(Permission::ReadBackend)
    }

    fn sync_article_authority(&mut self) -> bool {
        let current = self.can_read_articles().then(|| Authority {
            connection_epoch: self.connection.epoch,
            origin_revision: self.origin_revision(),
        });
        let changed =
            self.search_articles.authority.is_some() && self.search_articles.authority != current;
        if changed {
            self.search_articles.reset();
        }
        self.search_articles.authority = current;
        changed
    }

    /// Update the indexed query after a shared search input edit.
    pub fn edit_article_search(&mut self, query: &str, now: f64) {
        self.sync_article_authority();
        let query = query.trim();
        self.search_articles.change_scope(None);
        if query.is_empty() {
            return;
        }
        if query.len() > MAX_QUERY_BYTES
            || query.chars().any(char::is_control)
            || !query.chars().any(char::is_alphanumeric)
        {
            self.search_articles.status =
                Load::Failed("Enter a search with letters or numbers, up to 256 bytes.".into());
            return;
        }
        self.search_articles.scope = Some(ArticleScope::Search(query.to_owned()));
        self.search_articles.pending_at = Some(now + DEBOUNCE_SECONDS);
        self.search_articles.status = Load::Pending;
    }

    pub(crate) fn poll_article_search(&mut self, now: f64) -> bool {
        let changed = self.sync_article_authority();
        if self.search_articles.pending_at.is_some_and(|at| now >= at) {
            self.request_article_page(false);
            return true;
        }
        changed
    }

    /// Apply the UTC run day through the same list endpoint on either transport.
    pub fn browse_articles(&mut self) {
        self.sync_article_authority();
        let day = self.search_articles.browse_day.text.trim();
        if day.len() != 10 || NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() {
            self.search_articles.change_scope(None);
            self.search_articles.status = Load::Failed("Enter a date in YYYY-MM-DD format.".into());
            return;
        }
        let scope = ArticleScope::Browse(day.to_owned());
        if self.search_articles.scope.as_ref() == Some(&scope)
            && !matches!(self.search_articles.status, Load::Idle)
        {
            return;
        }
        self.search_articles.change_scope(Some(scope));
        self.request_article_page(false);
    }

    pub fn retry_articles(&mut self) {
        if self.search_articles.can_retry() {
            self.request_article_page(false);
        }
    }

    pub fn load_more_articles(&mut self) {
        if self.search_articles.can_load_more() {
            self.request_article_page(true);
        }
    }

    fn request_article_page(&mut self, tail: bool) {
        self.sync_article_authority();
        if self.search_articles.authority.is_none() {
            self.search_articles.reset();
            self.search_articles.status =
                Load::Failed("A connected backend with article access is required.".into());
            return;
        }
        let Some(ticket) = self.search_articles.start(tail) else {
            return;
        };
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            self.search_articles
                .complete(ticket, Err(ClientError::Transport));
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            self.search_articles
                .complete(ticket, Err(ClientError::Transport));
            return;
        };
        self.search_articles.request = Some(services.read_articles(ui, client.clone(), ticket));
    }
}

#[derive(Serialize)]
struct BrowseQuery<'a> {
    day: &'a str,
    category: &'static str,
    search: &'static str,
    sort: &'static str,
    limit: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<&'a str>,
}

impl<'a> BrowseQuery<'a> {
    fn new(day: &'a str, cursor: Option<&'a str>) -> Self {
        Self {
            day,
            category: "all",
            search: "",
            sort: "latest",
            limit: PAGE_SIZE,
            cursor,
        }
    }
}

async fn read(client: &Client, ticket: &ArticleRequest) -> Result<ArticlePage, ClientError> {
    let mut page: ArticlePage = match &ticket.scope {
        ArticleScope::Browse(day) => {
            let query = BrowseQuery::new(day, ticket.cursor.as_deref());
            client.get_with_query(ARTICLES_PATH, &query).await?
        }
        ArticleScope::Search(term) => {
            let mut query = ArticlesSearchQuery::default();
            query.query = term.clone();
            query.limit = Some(PAGE_SIZE);
            query.cursor = ticket.cursor.clone();
            client.get_with_query(ARTICLES_SEARCH_PATH, &query).await?
        }
    };
    if page.rows.len() > usize::from(PAGE_SIZE)
        || page
            .next_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
    {
        return Err(ClientError::InvalidResponse);
    }
    for row in &mut page.rows {
        if [&row.id, &row.run_id]
            .into_iter()
            .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
        {
            return Err(ClientError::InvalidResponse);
        }
        row.title = display_text(&row.title, 256);
        row.source_name = display_text(&row.source_name, 96);
        row.published_at = display_text(&row.published_at, 48);
        row.excerpt = row.excerpt.as_deref().map(|text| display_text(text, 480));
    }
    Ok(page)
}

fn display_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect()
}

impl GatewayEffects {
    fn read_articles(
        &self,
        ui: rune::AppHandle<State>,
        client: Client,
        ticket: ArticleRequest,
    ) -> RequestTask {
        let task = self.runtime.spawn(async move {
            let result = read(&client, &ticket).await;
            ui.update(move |state, _| {
                state.sync_article_authority();
                state.search_articles.complete(ticket, result);
            });
        });
        RequestTask(task.abort_handle())
    }
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::IpcOperation;

    use super::*;

    fn page(ids: &[&str], cursor: Option<&str>) -> ArticlePage {
        ArticlePage {
            rows: ids
                .iter()
                .map(|id| ArticleRow {
                    id: (*id).into(),
                    run_id: "test-run".into(),
                    title: format!("Article {id}"),
                    source_name: "Test source".into(),
                    published_at: "2026-11-27T10:00:00Z".into(),
                    excerpt: None,
                })
                .collect(),
            next_cursor: cursor.map(str::to_owned),
            truncated: false,
        }
    }

    #[test]
    fn browse_day_and_opaque_continuation_decode_through_local_ipc() {
        let cursor = r#"["2026-11-27","2026-11-27","article","test-run"]"#;
        let op = IpcOperation::from_get_with_query(
            ARTICLES_PATH,
            &BrowseQuery::new("2026-11-27", Some(cursor)),
        )
        .unwrap();
        let IpcOperation::ArticlesList(query) = op else {
            panic!("day browse must use the typed article list");
        };
        assert_eq!(query.day, "2026-11-27");
        assert_eq!(query.category, "all");
        assert_eq!(query.cursor.as_deref(), Some(cursor));
        assert_eq!(query.limit, PAGE_SIZE);
    }

    #[test]
    fn tail_retries_keep_rows_and_a_replaced_query_drops_its_reply() {
        let mut search = ArticlesSearch {
            authority: Some(Authority {
                connection_epoch: 1,
                origin_revision: 2,
            }),
            scope: Some(ArticleScope::Search("one".into())),
            ..ArticlesSearch::default()
        };
        let head = search.start(false).unwrap();
        search.complete(head, Ok(page(&["a"], Some("20"))));
        let tail = search.start(true).unwrap();
        search.complete(tail, Err(ClientError::Timeout));
        assert!(search.tail_error().is_some());
        assert!(search.can_load_more());

        let retry = search.start(true).unwrap();
        assert_eq!(retry.cursor.as_deref(), Some("20"));
        search.complete(retry, Ok(page(&["a", "b"], Some("20"))));
        let Load::Ready(results) = &search.status else {
            panic!("a tail retry must preserve the loaded list");
        };
        assert_eq!(results.rows.len(), 2);
        assert_eq!(results.rows[0].id, "a");
        assert_eq!(results.rows[1].id, "b");
        assert!(results.truncated);
        assert!(!search.can_load_more(), "a repeated cursor must stop");

        search.change_scope(Some(ArticleScope::Search("two".into())));
        let superseded = search.start(false).unwrap();
        search.change_scope(Some(ArticleScope::Browse("2026-11-27".into())));
        let current = search.start(false).unwrap();
        search.complete(superseded, Ok(page(&["old"], None)));
        assert!(matches!(search.status, Load::Loading));
        search.complete(current, Ok(page(&["day"], None)));
        let Load::Ready(results) = &search.status else {
            panic!("the current browse response must be accepted");
        };
        assert_eq!(results.rows[0].id, "day");
    }
}
