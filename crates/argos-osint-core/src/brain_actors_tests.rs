use super::*;
use crate::memory_metadata::ActorSubtype;
use crate::store::Store;

fn req<'a>(name: &'a str, scope: &'a str) -> BindRequest<'a> {
    BindRequest {
        name,
        subtype: ActorSubtype::Person,
        scope,
        identifiers: &[],
        external: None,
        provenance: json!({}),
    }
}

fn memory(store: &Store, text: &str, conversation: &str) -> String {
    store
        .add_memory(
            text,
            "fact",
            false,
            crate::brain::MemorySource {
                app: "recon".into(),
                conversation_id: conversation.into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap()
        .id
}
