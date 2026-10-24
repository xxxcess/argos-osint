//! Classification-only cycle regressions. Historical Brain maintenance is
//! independently covered by atlas_memory's direct-helper tests.
#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use crate::atlas::{run_atlas, AtlasEvent, Cursor, RunInput, RunStats, Stop};
    use crate::store::{AtlasArticleRow, Store};

    const RUN: &str = "classification-contract-fixture";

    async fn resume(path: &std::path::Path) -> (Stop, Vec<AtlasEvent>) {
        let mut events = Vec::new();
        let pause = AtomicBool::new(false);
        let keys = crate::osint::ProviderKeys::default();
        let stop = run_atlas(
            RunInput {
                db_path: path,
                pause: &pause,
                keys: &keys,
                user_agent: "Argos offline fixture",
                pace: false,
                resume: true,
                feed: &[],
                classifier: None,
                synthesizer: None,
                synthesizer_fallbacks: &[],
                run_id: Some(RUN),
            },
            |event| events.push(event),
            |_call| {
                Box::pin(async {
                    panic!("retained classification/native resume must not fetch provider data")
                })
            },
        )
        .await
        .unwrap();
        (stop, events)
    }
}
