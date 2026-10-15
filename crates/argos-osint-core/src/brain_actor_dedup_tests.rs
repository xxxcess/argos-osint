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
fn mention(s: &Store, id: &str, name: &str, article: &str) {
    crate::atlas_actor_index::tests::seed(
        s,
        "r",
        article,
        &format!("{name} chaired the Acme board"),
        &[name],
    );
    let a = s.atlas_article("r", article).unwrap().unwrap();
    record_mention(
        &s.conn,
        id,
        "article",
        "r",
        article,
        &crate::atlas_metadata::input_hash(&a),
        &json!([]),
        &chrono::Utc::now().to_rfc3339(),
    )
    .unwrap();
}
fn lease(s: &Store, c: &Candidate) {
    s.conn.execute("INSERT OR REPLACE INTO brain_actor_dedup_receipts(pair_key,input_hash,lease_owner,lease_until,updated_at) VALUES(?1,?2,'worker','2099-01-01','now')",params![c.key(),c.input_hash]).unwrap();
}
#[test]
fn possessives_normalize_before_article_thread_and_claim_binding() {
    let s = Store::memory().unwrap();
    for base in ["Trump", "South Africa", "Manchester United", "Marcos Jr"] {
        let id = actor(&s, base, "same");
        for suffix in ["'s", "’s", "ʼs"] {
            assert_eq!(actor(&s, &format!("{base}{suffix}"), "same"), id);
        }
    }
    assert_eq!(actors::normalize_name(" O'Connor’s "), "O'Connor");
    assert_eq!(actors::normalize_name("James’"), "James");
    assert_ne!(actor(&s, "Trump", "other"), actor(&s, "Trump", "same"));
    crate::atlas_actor_index::tests::seed(
        &s,
        "r",
        "a",
        "Trump and Trump's office met Trump’s aides",
        &["Trump", "Trump's", "Trump’s"],
    );
    let a = s.atlas_article("r", "a").unwrap().unwrap();
    let metadata = s.atlas_metadata("r", "a").unwrap().unwrap();
    let found = crate::atlas_metadata::article_actors(&a, &metadata);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].mentions.len(), 3);
    assert_eq!(found[0].spellings.len(), 3);
}
#[test]
fn local_repair_preserves_ids_and_merges_only_same_source_spellings() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Trump", "same");
    let b = actor(&s, "Temporary", "same");
    let c = actor(&s, "Trump", "different");
    s.conn
        .execute(
            "UPDATE brain_actors SET display_name='Trump’s' WHERE actor_id=?1",
            [&b],
        )
        .unwrap();
    repair_local(&s, 100).unwrap();
    assert_eq!(
        actors::resolve(&s.conn, &a).unwrap(),
        actors::resolve(&s.conn, &b).unwrap()
    );
    assert_eq!(actors::resolve(&s.conn, &c).unwrap(), c);
    assert!(s.get_memory(&b).unwrap().is_some());
    // Survivor revision can need one extra receipt after a merge.
    repair_local(&s, 100).unwrap();
    assert_eq!(repair_local(&s, 100).unwrap(), 0);
}
#[test]
fn semantic_candidates_cache_decisions_and_preserve_merge_lineage() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Meghan", "one");
    let b = actor(&s, "Meghan Markle", "two");
    mention(&s, &a, "Meghan", "a");
    mention(&s, &b, "Meghan Markle", "b");
    let pairs = candidates(&s, 8).unwrap();
    assert_eq!(pairs.len(), 1);
    let c = &pairs[0];
    let adapter = IdentityDecision(c);
    adapter.preflight().unwrap();
    assert_eq!(adapter.compile_native().0["pair"], c.context);
    let native:crate::provider::DecisionsResponse=serde_json::from_value(json!({"answers":{"identity":{"type":"choice","choice":"same","probabilities":{"same":0.98,"different":0.01,"uncertain":0.01}}}})).unwrap();
    assert_eq!(adapter.parse_native(&native).unwrap(), "same");
    let invalid:crate::provider::DecisionsResponse=serde_json::from_value(json!({"answers":{"identity":{"type":"choice","choice":"same","probabilities":{"invented":1.0}}}})).unwrap();
    assert!(adapter.parse_native(&invalid).is_err());
    assert!(adapter.parse_chat(r#"{"identity":"maybe"}"#).is_err());
    assert!(adapter
        .parse_chat(r#"{"identity":"same","extra":true}"#)
        .is_err());
    assert!(adapter
        .parse_chat(r#"{"identity":"different","identity":"same"}"#)
        .is_err());
    lease(&s, c);
    assert!(!apply(&s, c, "same", "wrong", &json!({})).unwrap());
    assert!(apply(&s, c, "uncertain", "worker", &json!({})).unwrap());
    assert!(candidates(&s, 8).unwrap().is_empty());
    // Identical source text reuses the assessment; new identity context
    // invalidates it, independently of article counts.
    mention(&s, &a, "Meghan", "new");
    assert!(candidates(&s, 8).unwrap().is_empty());
    let mut article = s.atlas_article("r", "new").unwrap().unwrap();
    article.description = "Meghan is also named Meghan Markle, the Duchess of Sussex.".into();
    s.atlas_upsert_article(&article).unwrap();
    record_mention(
        &s.conn,
        &a,
        "article",
        "r",
        "new",
        &crate::atlas_metadata::input_hash(&article),
        &json!([]),
        "now",
    )
    .unwrap();
    let c = candidates(&s, 8).unwrap().remove(0);
    lease(&s, &c);
    assert!(apply(&s, &c, "same", "worker", &json!({"assessed":true})).unwrap());
    assert_eq!(
        actors::resolve(&s.conn, &a).unwrap(),
        actors::resolve(&s.conn, &b).unwrap()
    );
    assert_eq!(actors::mentions(&s.conn, &a).unwrap().len(), 3);
    let redirected = if actors::resolve(&s.conn, &c.left).unwrap() != c.left {
        &c.left
    } else {
        &c.right
    };
    s.split_actor(redirected).unwrap();
    assert!(
        candidates(&s, 8).unwrap().is_empty(),
        "explicit split is respected"
    );
    assert_ne!(
        actors::resolve(&s.conn, &a).unwrap(),
        actors::resolve(&s.conn, &b).unwrap()
    );
}
#[test]
fn stale_source_and_expired_leases_cannot_merge() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Jane Doe", "one");
    let b = actor(&s, "Jane Doe", "two");
    mention(&s, &a, "Jane Doe", "a");
    mention(&s, &b, "Jane Doe", "b");
    let c = candidates(&s, 8).unwrap().remove(0);
    lease(&s, &c);
    s.atlas_delete_article("r", "a").unwrap();
    assert!(!apply(&s, &c, "same", "worker", &json!({})).unwrap());
    mention(&s, &a, "Jane Doe", "a");
    let c = candidate(&s, &c.left, &c.right).unwrap();
    lease(&s, &c);
    s.conn
        .execute_batch("UPDATE brain_actor_dedup_receipts SET lease_until='2000-01-01'")
        .unwrap();
    assert!(!apply(&s, &c, "same", "worker", &json!({})).unwrap());
    assert_ne!(
        actors::resolve(&s.conn, &a).unwrap(),
        actors::resolve(&s.conn, &b).unwrap()
    );
}

#[tokio::test]
async fn maintenance_worker_uses_native_or_chat_decisions_and_reuses_receipts() {
    use crate::provider_attempt::mock;
    for native in [true, false] {
        let reply = if native {
            mock::Reply::Json(200,json!({"id":"identity-1","model":"typesafe/jev-1.13","answers":{"identity":{"type":"choice","choice":"same","probabilities":{"same":0.98,"different":0.01,"uncertain":0.01}}}}).to_string())
        } else {
            mock::ok_json(r#"{"identity":"same"}"#, "stop")
        };
        let server = mock::serve(vec![reply]).await;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        let s = Store::open_initialized_light(&db).unwrap();
        let a = actor(&s, "Meghan", "one");
        let b = actor(&s, "Meghan Markle", "two");
        mention(&s, &a, "Meghan", "a");
        mention(&s, &b, "Meghan Markle", "b");
        drop(s);
        let provider = if native { "openrouter" } else { "local" };
        let model = if native {
            "typesafe/jev-1.13"
        } else {
            "fixture"
        };
        let mut auth = crate::secrets::AuthFile::default();
        auth.set_account(crate::secrets::ProviderSecret {
            kind: provider.into(),
            model: model.into(),
            base_url: server.base_url.clone(),
            api_key: Some("fixture-only".into()),
            stt_model: None,
            device: None,
        });
        let mut settings = crate::provider::SettingsFile::default();
        settings.defaults.entity_resolver = crate::provider::ModelAssignment {
            provider: provider.into(),
            model: model.into(),
            ..Default::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            drain_with(
                &db,
                "worker",
                &cancel,
                &crate::secrets::AuthFile::default(),
                &crate::provider::SettingsFile::default()
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(server.hits(), 0, "local repair needs no resolver");
        assert_eq!(
            drain_with(&db, "worker", &cancel, &auth, &settings)
                .await
                .unwrap(),
            1
        );
        assert_eq!(server.hits(), 1);
        assert_eq!(
            drain_with(&db, "worker", &cancel, &auth, &settings)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            server.hits(),
            1,
            "unchanged decisions issue no repeat requests"
        );
        let s = Store::open_light(&db).unwrap();
        assert_eq!(
            actors::resolve(&s.conn, &a).unwrap(),
            actors::resolve(&s.conn, &b).unwrap()
        );
        assert_eq!(
            s.actor_group("meghan markle")
                .unwrap()
                .unwrap()
                .actor_ids
                .len(),
            2
        );
        assert!(s
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM argos_jobs WHERE title='Actor identity maintenance')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
        let body: Value = serde_json::from_str(
            server.requests.lock().unwrap()[0]
                .split("\r\n\r\n")
                .last()
                .unwrap(),
        )
        .unwrap();
        if native {
            assert!(body["questions"]["identity"].is_object());
            assert!(body.get("messages").is_none());
        } else {
            assert!(body["messages"].is_array());
        }
    }
}

#[test]
fn assessed_aliases_and_redirect_aliases_propose_candidates_without_name_overlap() {
    let s = Store::memory().unwrap();
    let original = actor(&s, "Twitter", "one");
    for name in ["Twitter", "X"] {
        s.bind_actor(&BindRequest {
            name,
            scope: "one",
            subtype: ActorSubtype::Person,
            identifiers: &[],
            external: Some(("fixture", "legacy")),
            provenance: json!({}),
        })
        .unwrap();
    }
    let fuller = actor(&s, "Platform X", "two");
    s.merge_actors(&original, &fuller, &json!({"assessed":true}))
        .unwrap();
    let candidate = actor(&s, "X", "three");
    mention(&s, &fuller, "Platform X", "a");
    mention(&s, &candidate, "X", "b");
    let pairs = candidates(&s, 8).unwrap();
    assert_eq!(pairs.len(), 1);
    assert!(pairs[0].left == candidate || pairs[0].right == candidate);
    assert!(pairs[0].left == fuller || pairs[0].right == fuller);
}

#[test]
fn split_separations_follow_later_survivor_changes() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Jane Doe", "one");
    let b = actor(&s, "Jane Doe", "two");
    let c = actor(&s, "Jane Doe", "three");
    mention(&s, &a, "Jane Doe", "a");
    mention(&s, &b, "Jane Doe", "b");
    mention(&s, &c, "Jane Doe", "c");
    s.merge_actors(&b, &a, &json!({"assessed":true})).unwrap();
    s.split_actor(&b).unwrap();
    s.merge_actors(&a, &c, &json!({"assessed":true})).unwrap();
    assert!(
        candidates(&s, 8).unwrap().is_empty(),
        "a third survivor must not circumvent an explicit separation"
    );
}

#[test]
fn fresh_article_context_can_reconsider_an_older_uncertain_identity() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Meghan", "one");
    let b = actor(&s, "Meghan Markle", "two");
    mention(&s, &b, "Meghan Markle", "b");
    for n in 0..4 {
        mention(&s, &a, &format!("Meghan {n}"), &format!("a{n}"));
    }
    let before = candidates(&s, 8).unwrap().remove(0);
    lease(&s, &before);
    apply(&s, &before, "uncertain", "worker", &json!({})).unwrap();
    assert!(candidates(&s, 8).unwrap().is_empty());
    mention(&s, &a, "Meghan Markle", "new");
    let after = candidates(&s, 8).unwrap().remove(0);
    assert_ne!(before.input_hash, after.input_hash);
    assert!(after.context.to_string().contains("Meghan Markle chaired"));
}
