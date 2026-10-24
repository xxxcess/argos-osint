//! Runtime context a Body stage task passes to article body retrieval and
//! cleanup. The retired Intel report-job worker that used to live here is gone.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::osint::ProviderKeys;
use crate::provider::SettingsFile;
use crate::secrets::{AuthFile, ProviderSecret};

#[derive(Clone)]
pub struct JobRuntime {
    /// Owning brief stage execution, when this runtime serves a Body task
    /// (cleanup attempts attach to that task and epoch).
    pub stage: Option<super::brief_stages::StageContext>,
    pub db_path: PathBuf,
    pub job_id: String,
    pub article_title: String,
    pub article_url: String,
    pub article_preview: String,
    pub article_published: String,
    pub article_domain: String,
    pub run_id: String,
    pub keys: ProviderKeys,
    pub auth: AuthFile,
    pub synthesis_secret: Option<ProviderSecret>,
    pub classifier_secret: Option<ProviderSecret>,
    pub settings: SettingsFile,
    pub cancel: Arc<AtomicBool>,
}
