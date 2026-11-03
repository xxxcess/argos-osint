/// Seed only the catalog identities owned by an offline storage fixture.
pub async fn seed_actor_catalog(
    repo: &argos_osint_core::storage::repo::Repository,
    names: &[&str],
) {
    let actors = names
        .iter()
        .map(|name| argos_osint_core::actor_catalog::Actor {
            id: format!("fixture-{name}"),
            source_id: format!("fixture:{name}"),
            name: (*name).into(),
            subtype: "person".into(),
            aliases: vec![(*name).into()],
            metadata: Default::default(),
        })
        .collect();
    argos_osint_core::storage::actor_catalog::seed_actors(
        repo,
        actors,
        "offline-storage-fixture",
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed fixture actor catalog");
}
