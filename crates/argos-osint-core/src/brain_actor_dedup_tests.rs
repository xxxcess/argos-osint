use super::*;
use crate::brain_actors::{record_mention, BindRequest};
use crate::memory_metadata::ActorSubtype;

fn actor(s: &Store, name: &str, scope: &str) -> String {
    s.bind_actor(&BindRequest {
        name,
        scope,
        subtype: ActorSubtype::Person,
        identifiers: &[],
        external: None,
        provenance: json!({}),
    })
    .unwrap()
    .actor_id
}
