use super::*;
use crate::brain_actors::{add_memory_actor_ref, BindRequest};
use crate::memory_metadata::{ActorSubtype, IdentityState};
use serde_json::json;

fn actor(store: &Store, name: &str, scope: &str) -> String {
    store
        .bind_actor(&BindRequest {
            name,
            subtype: ActorSubtype::Person,
            scope,
            identifiers: &[],
            external: None,
            provenance: json!({"test":true}),
        })
        .unwrap()
        .actor_id
}

fn claim(store: &Store, text: &str) -> Memory {
    let m = store
        .add_memory(
            text,
            "fact",
            false,
            MemorySource {
                app: "user".into(),
                conversation_id: "test".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    store.mark_user_claim(&m.id).unwrap();
    m
}
