//! Registered Inbox QA operations on the selected gateway or local companion.

use super::Client;
use super::ClientError;
use crate::chat::QaCancelRequest;
use crate::chat::QaEventPage;
use crate::chat::QaEventQuery;
use crate::chat::QaRelatedActivity;
use crate::chat::QaRelatedActivityRequest;
use crate::chat::QaSettings;
use crate::chat::QaSettingsUpdate;
use crate::chat::QaTurn;
use crate::chat::QaTurnRequest;
use crate::chat::QA_CANCEL_PATH;
use crate::chat::QA_EVENTS_PATH;
use crate::chat::QA_HISTORY_PATH;
use crate::chat::QA_LATEST_PATH;
use crate::chat::QA_MAX_RETAINED_TURNS;
use crate::chat::QA_RELATED_ACTIVITY_PATH;
use crate::chat::QA_SETTINGS_PATH;
use crate::chat::QA_TURNS_PATH;

pub(super) fn is_qa_route(route: &str) -> bool {
    matches!(
        route,
        QA_SETTINGS_PATH
            | QA_TURNS_PATH
            | QA_LATEST_PATH
            | QA_HISTORY_PATH
            | QA_EVENTS_PATH
            | QA_CANCEL_PATH
            | QA_RELATED_ACTIVITY_PATH
    )
}

impl Client {
    fn require_qa_registration(&self) -> Result<(), ClientError> {
        self.registered_context
            .as_ref()
            .map(|_| ())
            .ok_or(ClientError::InvalidRequest)
    }

    /// Read inference state and the registration's management permission.
    pub async fn qa_settings(&self) -> Result<QaSettings, ClientError> {
        self.require_qa_registration()?;
        self.get(QA_SETTINGS_PATH).await
    }

    /// Change the selected backend's inference state using its current revision.
    pub async fn update_qa_settings(
        &self,
        request: &QaSettingsUpdate,
    ) -> Result<QaSettings, ClientError> {
        self.require_qa_registration()?;
        self.post(QA_SETTINGS_PATH, request).await
    }

    /// Submit a question or replay its original accepted request ID.
    pub async fn submit_qa_turn(&self, request: &QaTurnRequest) -> Result<QaTurn, ClientError> {
        self.require_qa_registration()?;
        self.post(QA_TURNS_PATH, request).await
    }

    /// Resolve related activity without accepting a turn or reserving a job.
    pub async fn qa_related_activity(
        &self,
        request: &QaRelatedActivityRequest,
    ) -> Result<QaRelatedActivity, ClientError> {
        self.require_qa_registration()?;
        request
            .validate()
            .map_err(|_| ClientError::InvalidRequest)?;
        self.post(QA_RELATED_ACTIVITY_PATH, request).await
    }

    /// Read the latest accepted exchange for this registered workstation.
    pub async fn latest_qa_turn(&self) -> Result<Option<QaTurn>, ClientError> {
        self.require_qa_registration()?;
        self.get(QA_LATEST_PATH).await
    }

    /// Read retained terminal exchanges for this registration, newest first.
    pub async fn qa_history(&self) -> Result<Vec<QaTurn>, ClientError> {
        self.require_qa_registration()?;
        let turns: Vec<QaTurn> = self.get(QA_HISTORY_PATH).await?;
        if turns.len() > QA_MAX_RETAINED_TURNS
            || turns
                .iter()
                .any(|turn| !turn.state.is_terminal() || turn.provisional_reply.is_some())
        {
            return Err(ClientError::InvalidResponse);
        }
        Ok(turns)
    }

    /// Resume the selected turn from a cursor, waiting briefly for new events when requested.
    pub async fn qa_events(&self, query: &QaEventQuery) -> Result<QaEventPage, ClientError> {
        self.require_qa_registration()?;
        self.get_with_query(QA_EVENTS_PATH, query).await
    }

    /// Stop an accepted turn and return the exchange with any committed findings.
    pub async fn cancel_qa_turn(&self, request: &QaCancelRequest) -> Result<QaTurn, ClientError> {
        self.require_qa_registration()?;
        self.post(QA_CANCEL_PATH, request).await
    }
}
