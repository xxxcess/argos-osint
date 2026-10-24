use super::*;
use crate::atlas_actor_index::tests::seed;

fn index(store: &Store, run: &str) {
    assert!(crate::atlas_actor_index::index_run(store, run, &|| false)
        .unwrap()
        .complete());
}
