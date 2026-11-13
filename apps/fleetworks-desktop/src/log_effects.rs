//! UI-owned finite log reads and explicit local copy/save effects.

use std::path::PathBuf;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::FileDialogOptions;
use rune::Tx;

use crate::app::Intent;
use crate::app::Page;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::logs;
use crate::logs::ActivityTab;
use crate::logs::LogBinding;
use crate::logs::LogEvent;
use crate::logs::LogReply;
use crate::logs::LogTicket;

impl State {
    pub fn reveal_operational_logs(&mut self) {
        self.page = Page::Jobs;
        self.select_activity_tab(ActivityTab::Logs);
        self.sync_log_binding();
        self.reveal_control("scoped-operational-logs");
    }

    pub fn select_activity_tab(&mut self, tab: ActivityTab) {
        self.activity_tab = tab;
        self.sync_log_binding();
        self.sync_log_visibility();
    }

    pub(crate) fn refresh_operational_logs(&mut self) {
        self.sync_log_binding();
        self.sync_log_visibility();
        if !self.scoped_logs.visible || self.backend.selected_job.is_none() {
            return;
        }
        if let Some(ticket) = self.scoped_logs.refresh() {
            self.start_scoped_logs(ticket);
        }
    }

    pub(crate) fn selected_log_job_changed(&mut self, id: &str) {
        self.sync_log_binding();
        self.scoped_logs.select_target(crate::logs::LogTarget::Job {
            id: id.into(),
            descendants: false,
        });
        self.refresh_operational_logs();
    }
    fn log_binding(&self) -> Option<LogBinding> {
        if !self.connected() || !self.permission(Permission::ReadWorkstationActivity) {
            return None;
        }
        Some(LogBinding {
            context: self.registration.context.clone()?,
            connection_epoch: self.connection.epoch,
            origin_revision: self.origin_revision(),
        })
    }

    pub fn sync_log_binding(&mut self) {
        let binding = self.log_binding();
        let permitted = self.permission(Permission::ReadWorkstationActivity);
        self.scoped_logs.bind(binding, permitted);
    }

    pub(crate) fn sync_log_visibility(&mut self) {
        self.scoped_logs.set_visible(
            self.page == Page::Jobs
                && self.backend.selected_job.is_some()
                && self.scoped_logs.window_focused,
        );
    }

    pub(crate) fn log_window_focus(&mut self, focused: bool, _: &mut Tx) {
        self.scoped_logs.window_focused = focused;
        self.sync_log_visibility();
        if focused
            && self.scoped_logs.visible
            && !self.scoped_logs.available()
            && !self.scoped_logs.busy()
        {
            self.refresh_operational_logs();
        }
    }

    pub(crate) fn poll_scoped_logs(&mut self) -> bool {
        let previous = self.scoped_logs.revision();
        let visible = self.scoped_logs.visible;
        self.sync_log_binding();
        self.sync_log_visibility();
        if let Some(ticket) = self.scoped_logs.follow() {
            self.start_scoped_logs(ticket);
            return true;
        }
        previous != self.scoped_logs.revision() || visible != self.scoped_logs.visible
    }

    pub fn log_event(&mut self, event: LogEvent, tx: &mut Tx) {
        self.sync_log_binding();
        self.sync_log_visibility();
        let ticket = match event {
            LogEvent::Select(target) => {
                self.scoped_logs.select_target(target);
                None
            }
            LogEvent::Refresh => self.scoped_logs.refresh(),
            LogEvent::Older => self.scoped_logs.older(),
            LogEvent::ToggleFollow => {
                self.scoped_logs.set_following(!self.scoped_logs.following);
                self.scoped_logs.follow()
            }
            LogEvent::PrepareExport => self.scoped_logs.prepare_export(),
            LogEvent::ContinueExport => self.scoped_logs.continue_export(),
            LogEvent::CopyLoaded => {
                if let Some(copy) = self.scoped_logs.copy_loaded() {
                    if self.fixture {
                        self.record(Intent::CopyLogSummaries);
                    } else {
                        tx.copy(copy.text);
                    }
                    self.scoped_logs.message = Some(format!(
                        "Copied {} loaded safe summaries ({} eligible private scope{}).",
                        copy.rows,
                        if copy.complete_eligible_scope {
                            "complete observed"
                        } else {
                            "partial"
                        },
                        if copy.excludes_shared_private {
                            "; shared private logs excluded"
                        } else {
                            ""
                        }
                    ));
                }
                None
            }
            LogEvent::SaveExport { allow_partial } => {
                self.choose_log_export(allow_partial, tx);
                None
            }
        };
        if let Some(ticket) = ticket {
            self.start_scoped_logs(ticket);
        }
    }

    fn start_scoped_logs(&mut self, ticket: LogTicket) {
        self.record(Intent::OperationalLogs {
            action: ticket.action,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            self.scoped_logs
                .complete(&ticket, Err(ClientError::Transport));
            return;
        };
        self.scoped_logs.request = services.scoped_logs(client.clone(), ticket);
    }

    pub fn complete_scoped_logs(
        &mut self,
        ticket: LogTicket,
        result: Result<LogReply, ClientError>,
    ) {
        self.sync_log_binding();
        self.sync_log_visibility();
        self.scoped_logs.complete(&ticket, result);
        self.scoped_logs.set_following(self.scoped_logs.visible);
        // Poll owns resubscription. A fixture completion remains finite and
        // never recursively invents a stream of backend events.
    }

    fn choose_log_export(&mut self, allow_partial: bool, tx: &mut Tx) {
        if !self.scoped_logs.can_export()
            || self.scoped_logs.export.rows() == 0
            || self
                .scoped_logs
                .export
                .bytes_for_save(allow_partial)
                .is_none()
        {
            return;
        }
        let Some(binding) = self.log_binding() else {
            return;
        };
        let revision = (
            self.scoped_logs.revision(),
            self.scoped_logs.request_revision(),
        );
        let partial = !self.scoped_logs.export.complete;
        if self.fixture {
            self.record(Intent::SaveLogExport { partial });
            self.scoped_logs.message = Some(
                if partial {
                    "Fixture selected a labelled partial export; no file was written."
                } else {
                    "Fixture selected a complete eligible snapshot; no file was written."
                }
                .into(),
            );
            return;
        }
        tx.save_file(
            FileDialogOptions {
                title: if partial {
                    "Save partial safe log summaries"
                } else {
                    "Save complete eligible log snapshot"
                }
                .into(),
                file_name: Some(
                    if partial {
                        "fleetworks-logs.partial.ndjson"
                    } else {
                        "fleetworks-logs.ndjson"
                    }
                    .into(),
                ),
                ..Default::default()
            },
            move |state: &mut State, result, _| {
                state.sync_log_binding();
                if state.log_binding().as_ref() != Some(&binding)
                    || (
                        state.scoped_logs.revision(),
                        state.scoped_logs.request_revision(),
                    ) != revision
                {
                    return;
                }
                match result {
                    Ok(Some(path)) => {
                        let Some(bytes) = state
                            .scoped_logs
                            .export
                            .bytes_for_save(allow_partial)
                            .map(<[u8]>::to_vec)
                        else {
                            return;
                        };
                        let Some(services) = &state.services else {
                            return;
                        };
                        state.scoped_logs.request = services.save_scoped_logs(
                            path,
                            bytes,
                            binding.clone(),
                            revision,
                            partial,
                        );
                    }
                    Ok(None) => {}
                    Err(_) => {
                        state.scoped_logs.message =
                            Some("Log export destination chooser failed.".into())
                    }
                }
            },
        );
    }
}

impl GatewayEffects {
    fn scoped_logs(&self, client: Client, ticket: LogTicket) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = logs::execute(client, &ticket).await;
            ui.update(move |state, _| state.complete_scoped_logs(ticket, result));
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn save_scoped_logs(
        &self,
        path: PathBuf,
        bytes: Vec<u8>,
        binding: LogBinding,
        revision: (u64, u64),
        partial: bool,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task=self.runtime.spawn(async move {
            let result=tokio::task::spawn_blocking(move ||crate::transfer::write_private_export(path,bytes)).await;
            ui.update(move |state,_| {
                state.sync_log_binding();
                if state.log_binding().as_ref()!=Some(&binding)
                    || (state.scoped_logs.revision(),state.scoped_logs.request_revision())!=revision {return;}
                state.scoped_logs.request=None;
                state.scoped_logs.message=Some(match result {
                    Ok(Ok(()))=>if partial {"Created a new private partial log export (mode 0600)."}else{"Created a new private complete eligible log snapshot (mode 0600)."}.into(),
                    Ok(Err(error))=>error,
                    Err(_)=>"Log export writer stopped; review the selected destination before retrying.".into(),
                });
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}
