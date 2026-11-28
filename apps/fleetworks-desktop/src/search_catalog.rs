//! Search category navigation and bounded actor catalog reads.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::search::ActorBrowsePage;
use fleetworks_contracts::search::ActorBrowseQuery;
use fleetworks_contracts::search::ActorSearchHit;
use fleetworks_contracts::search::ACTOR_BROWSE_PATH;

use crate::app::Intent;
use crate::app::Page;
use crate::app::State;
use crate::connection::RequestTask;

const PAGE_SIZE: u16 = 10;
const MAX_ROWS: usize = 480;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchCategory {
    Actors,
    Articles,
}

#[derive(Default)]
pub(crate) struct ActorCatalog {
    pub(crate) rows: Vec<ActorSearchHit>,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    pub(crate) next: Option<String>,
    generation: u64,
    request: Option<RequestTask>,
}

impl ActorCatalog {
    pub(crate) fn reset(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }

    pub(crate) fn at_limit(&self) -> bool {
        self.rows.len() >= MAX_ROWS
    }

    pub(crate) fn continue_automatically(&self) -> bool {
        self.next.is_some() && !self.busy && !self.at_limit() && self.error.is_none()
    }
}

impl State {
    pub(crate) fn load_actor_catalog(&mut self) {
        let list = &mut self.actor_catalog;
        if self.page != Page::Brain
            || self.search_category != Some(SearchCategory::Actors)
            || list.busy
            || list.at_limit()
            || (!list.rows.is_empty() && list.next.is_none())
        {
            return;
        }
        if !self.connected() || !self.permission(Permission::ReadBackend) {
            self.actor_catalog.error =
                Some("Connect with access to read the actor catalog.".into());
            return;
        }
        let list = &mut self.actor_catalog;
        let Ok(query) = ActorBrowseQuery::new(list.next.clone(), PAGE_SIZE) else {
            list.error = Some("The actor catalog returned an invalid continuation.".into());
            list.next = None;
            return;
        };
        list.error = None;
        list.busy = true;
        let generation = list.generation;
        let cursor = list.next.clone();
        self.record(Intent::BrowseActors);
        if self.fixture {
            return;
        }
        let Some((services, client, ui)) = self.services.as_ref().and_then(|services| {
            Some((services, self.client.clone()?, services.ui.get()?.clone()))
        }) else {
            self.complete_actor_catalog(
                generation,
                cursor,
                Err("Actor catalog is unavailable.".into()),
            );
            return;
        };
        let epoch = self.connection.epoch;
        let origin = self.origin_revision();
        let task = services.runtime.spawn(async move {
            let result = client
                .get_with_query(ACTOR_BROWSE_PATH, &query)
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision() == origin {
                    state.complete_actor_catalog(generation, cursor, result);
                }
            });
        });
        self.actor_catalog.request = Some(RequestTask(task.abort_handle()));
    }

    /// Complete a browse request from synthetic native fixture data.
    pub fn fixture_actor_catalog(&mut self, page: ActorBrowsePage) {
        if self.fixture {
            let catalog = &self.actor_catalog;
            self.complete_actor_catalog(catalog.generation, catalog.next.clone(), Ok(page));
        }
    }

    fn complete_actor_catalog(
        &mut self,
        generation: u64,
        cursor: Option<String>,
        result: Result<ActorBrowsePage, String>,
    ) {
        let list = &mut self.actor_catalog;
        if list.generation != generation || !list.busy {
            return;
        }
        list.busy = false;
        list.request = None;
        match result {
            Ok(page) if page.rows.len() <= usize::from(PAGE_SIZE) => {
                let before = list.rows.len();
                for row in page.rows {
                    if !list.at_limit()
                        && !list.rows.iter().any(|prior| prior.actor_id == row.actor_id)
                    {
                        list.rows.push(row);
                    }
                }
                list.next = page.next_cursor.filter(|next| {
                    !next.is_empty() && cursor.as_ref() != Some(next) && list.rows.len() > before
                });
            }
            Ok(_) => list.error = Some("The actor catalog returned too many records.".into()),
            Err(error) => list.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::Env;
    use rune::Runtime;

    const RESULTS: &str = "search-category-results";
    const MORE: &str = "actor-catalog-more";

    fn page(ids: &[&str], next: Option<&str>) -> ActorBrowsePage {
        let rows = ids
            .iter()
            .map(|id| ActorSearchHit::new((*id).into(), (*id).into(), 0, String::new()))
            .collect();
        ActorBrowsePage::new(rows, next.map(str::to_owned))
    }

    fn open_catalog(viewport: (f32, f32)) -> Runtime<State> {
        let mut state = crate::tests::registered_fixture();
        state.page = Page::Brain;
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            viewport,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.reduced_motion = true;
        runtime.frame(0.0);
        assert!(runtime.find("search-landing-bar").is_some());
        let (x, y) = runtime
            .find("search-category-actors")
            .expect("Actors avatar");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        assert_eq!(runtime.state.search_category, Some(SearchCategory::Actors));
        assert!(runtime.find("search-landing-bar").is_none());
        assert!(runtime.state.actor_catalog.busy);
        runtime
    }

    fn actors(first: usize, count: usize, next: Option<&str>) -> ActorBrowsePage {
        let rows = (first..first + count)
            .map(|number| {
                ActorSearchHit::new(
                    format!("actor-{number}"),
                    format!("Actor {number}"),
                    0,
                    String::new(),
                )
            })
            .collect();
        ActorBrowsePage::new(rows, next.map(str::to_owned))
    }

    fn browse_requests(state: &State) -> usize {
        state
            .intents
            .iter()
            .filter(|intent| matches!(intent, Intent::BrowseActors))
            .count()
    }

    #[test]
    fn actor_category_switches_view_and_fences_browse_completions() {
        let mut runtime = open_catalog((1180.0, 780.0));
        runtime.update(|state, _| {
            state.fixture_actor_catalog(page(&["actor-1"], Some("actor-1")));
            state.load_actor_catalog();
            state.fixture_actor_catalog(page(&["actor-1", "actor-2"], None));
        });
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("actor-hit-actor-1").is_some());
        assert!(runtime.find("actor-hit-actor-2").is_some());
        assert_eq!(runtime.state.actor_catalog.rows.len(), 2);
        let old_generation = runtime.state.actor_catalog.generation;
        let (x, y) = runtime.find("search-back").expect("Back to search");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        runtime.update(move |state, _| {
            state.complete_actor_catalog(old_generation, None, Ok(page(&["late"], None)));
        });
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("search-landing-bar").is_some());
        assert!(runtime.state.actor_catalog.rows.is_empty());
    }

    #[test]
    fn catalog_that_fits_waits_for_load_more_and_allows_retry() {
        let mut runtime = open_catalog((1180.0, 1280.0));
        assert_eq!(browse_requests(&runtime.state), 1);
        runtime.update(|state, _| state.fixture_actor_catalog(actors(0, 10, Some("actor-9"))));
        runtime.frame(0.02);
        runtime.frame(0.04);
        assert_eq!(runtime.scroll_at_bottom(RESULTS), Some(true));
        assert_eq!(runtime.state.actor_catalog.rows.len(), 10);
        assert_eq!(browse_requests(&runtime.state), 1);
        assert!(!runtime.state.actor_catalog.busy);

        let (x, y) = runtime.find(MORE).expect("manual continuation");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.06);
        assert_eq!(browse_requests(&runtime.state), 2);
        runtime.update(|state, _| state.fixture_actor_catalog(actors(10, 11, Some("actor-20"))));
        runtime.frame(0.08);
        assert_eq!(runtime.state.actor_catalog.rows.len(), 10);
        assert_eq!(runtime.state.actor_catalog.next.as_deref(), Some("actor-9"));
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("The actor catalog returned too many records."));

        let (x, y) = runtime
            .find(MORE)
            .expect("retry after an oversized response");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.10);
        assert_eq!(browse_requests(&runtime.state), 3);
        runtime.update(|state, _| state.fixture_actor_catalog(actors(10, 10, None)));
        runtime.frame(0.12);
        assert_eq!(runtime.state.actor_catalog.rows.len(), 20);
        assert!(runtime.find(MORE).is_none());
    }

    #[test]
    fn scrolling_to_the_catalog_end_loads_one_batch_and_keeps_row_focus() {
        let mut runtime = open_catalog((640.0, 480.0));
        runtime.update(|state, _| state.fixture_actor_catalog(actors(0, 10, Some("actor-9"))));
        runtime.frame(0.02);
        runtime.frame(0.04);
        assert_eq!(runtime.scroll_at_bottom(RESULTS), Some(false));
        assert_eq!(browse_requests(&runtime.state), 1);

        let (x, y) = runtime.find(RESULTS).expect("catalog scroll viewport");
        runtime.wheel(x, y, -1000.0);
        runtime.frame(0.06);
        runtime.frame(0.08);
        assert_eq!(runtime.scroll_at_bottom(RESULTS), Some(true));
        assert_eq!(browse_requests(&runtime.state), 2);
        assert!(runtime.state.actor_catalog.busy);
        assert_eq!(runtime.state.actor_catalog.next.as_deref(), Some("actor-9"));

        let last_seen = "actor-hit-actor-9";
        runtime.update(|_, tx| tx.focus(last_seen, true));
        runtime.frame(0.10);
        runtime.update(|state, _| state.fixture_actor_catalog(actors(10, 10, Some("actor-19"))));
        runtime.frame(0.12);
        runtime.frame(0.14);
        assert_eq!(runtime.state.actor_catalog.rows.len(), 20);
        assert_eq!(browse_requests(&runtime.state), 2);
        assert!(runtime
            .elements()
            .into_iter()
            .any(|element| { element.key.as_deref() == Some(last_seen) && element.focused }));

        runtime.wheel(x, y, -1000.0);
        runtime.frame(0.16);
        runtime.frame(0.18);
        assert_eq!(browse_requests(&runtime.state), 3);
        assert_eq!(
            runtime.state.actor_catalog.next.as_deref(),
            Some("actor-19")
        );
        runtime.update(|state, _| state.fixture_actor_catalog(actors(20, 5, None)));
        runtime.frame(0.20);
        assert_eq!(runtime.state.actor_catalog.rows.len(), 25);
        assert!(runtime.find(MORE).is_none());
    }
}
