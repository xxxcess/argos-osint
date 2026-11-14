//! Finite registered Jobs reads and UI completion fences.

use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::Tx;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::backend::ActivityJobs;
use crate::backend::ReadKind;
use crate::backend::ReadRequest;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::jobs_query;
use crate::jobs_query::JobsBinding;
use crate::jobs_query::JobsEvent;
use crate::jobs_query::JobsLoad;
use crate::jobs_query::JobsPage;
use crate::jobs_query::JobsTicket;

impl State {
    /// Refresh proof/permission before requests and before accepting a reply.
    /// Logs keeps its own filters, loaded rows, cursor and finite-read owner.
    pub fn sync_jobs_binding(&mut self) -> bool {
        let permitted = self.permission(Permission::ReadWorkstationActivity);
        let binding = if self.connected() && permitted {
            self.registration
                .context
                .clone()
                .map(|context| JobsBinding {
                    context,
                    connection_epoch: self.connection.epoch,
                    origin_revision: self.origin_revision(),
                })
        } else {
            None
        };
        let revision = self.scoped_jobs.revision();
        self.scoped_jobs.bind(binding, permitted);
        self.scoped_jobs.draft.selected_workspace_only = self.backend.selected_workspace_only;
        let changed = revision != self.scoped_jobs.revision();
        if changed {
            self.clear_jobs_selection();
        }
        changed
    }

    fn clear_jobs_selection(&mut self) {
        self.backend.invalidate_activity();
        self.job_actions.confirmation = None;
        // Pending/uncertain action attempts retain their exact identity. This
        // view change neither cancels a backend job nor replaces an action.
    }

    pub(crate) fn refresh_scoped_jobs(&mut self) {
        self.request_jobs(JobsEvent::Apply);
    }

    pub fn jobs_event(&mut self, event: JobsEvent, _: &mut Tx) {
        self.request_jobs(event);
    }

    pub fn request_jobs(&mut self, event: JobsEvent) {
        self.sync_jobs_binding();
        let now = if self.fixture {
            DateTime::parse_from_rfc3339("2026-11-12T12:00:00Z")
                .expect("fixed synthetic fixture clock")
                .with_timezone(&Utc)
        } else {
            Utc::now()
        };
        let effect = self.scoped_jobs.handle(event, now);
        if effect.clear_selection {
            self.clear_jobs_selection();
        }
        let Some(ticket) = effect.read else {
            self.project_jobs_page();
            return;
        };
        let keep_ready_jobs = matches!(event, JobsEvent::Poll)
            && !effect.clear_selection
            && matches!(self.backend.jobs, Load::Ready(_));
        if !keep_ready_jobs {
            self.backend.jobs = Load::Loading;
        }
        self.record(Intent::BackendRead {
            kind: ReadKind::Jobs,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            self.complete_scoped_jobs(ticket, Err(ClientError::Transport));
            return;
        };
        self.scoped_jobs.request = services.scoped_jobs(client.clone(), ticket);
    }

    pub fn complete_scoped_jobs(
        &mut self,
        ticket: JobsTicket,
        result: Result<JobsPage, ClientError>,
    ) {
        self.sync_jobs_binding();
        if self.scoped_jobs.complete(&ticket, result) {
            self.project_jobs_page();
            if let (Some(selected_id), Load::Ready(jobs)) =
                (self.backend.selected_job.clone(), &self.backend.jobs)
            {
                if let Some(updated) = jobs.rows.iter().find(|job| job.id == selected_id).cloned() {
                    let transitioned = match &mut self.backend.detail {
                        Load::Ready(detail) if detail.job.id == updated.id => {
                            let changed = detail.job.state != updated.state
                                || detail.job.phase != updated.phase
                                || detail.job.progress_done != updated.progress_done
                                || detail.job.progress_total != updated.progress_total
                                || detail.job.error_summary != updated.error_summary;
                            if changed {
                                detail.job = updated;
                            }
                            changed
                        }
                        _ => false,
                    };
                    if transitioned {
                        if !self.fixture && !self.backend.in_flight(ReadKind::JobDetail) {
                            self.backend_read(ReadRequest::JobDetail { id: selected_id });
                        }
                        self.sync_operational_logs_with_selected_job(true);
                    }
                }
            }
        }
    }

    /// The existing bounded detail/action consumers see only this validated
    /// current-page projection. This is never a fallback legacy list request.
    fn project_jobs_page(&mut self) {
        self.backend.jobs = match (&self.scoped_jobs.page, self.scoped_jobs.status) {
            (Some(page), JobsLoad::Ready) => Load::Ready(ActivityJobs {
                rows: page.rows.clone(),
                activity_scope: page.activity_scope.clone(),
                relation_counts_available: page.relation_counts_available,
                activity_actions_available: page.activity_actions_available,
            }),
            (_, JobsLoad::Loading) => Load::Loading,
            (_, JobsLoad::Failed | JobsLoad::RestartRequired) => Load::Failed(
                self.scoped_jobs
                    .message
                    .clone()
                    .unwrap_or_else(|| "Jobs could not be loaded.".into()),
            ),
            _ => Load::Idle,
        };
    }
}

impl GatewayEffects {
    fn scoped_jobs(&self, client: Client, ticket: JobsTicket) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = jobs_query::read(&client, &ticket).await;
            ui.update(move |state, _| state.complete_scoped_jobs(ticket, result));
        });
        Some(RequestTask(task.abort_handle()))
    }
}
