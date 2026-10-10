//! Tests for the portable profile configuration transfer (spec section 3).
//!
//! Every test here is deterministic and offline: `ARGOS_EMBED` stays unset, the
//! `lancedb` feature is never enabled, and the only environment variables touched
//! are ones the test itself sets and then restores. API keys are obvious fakes
//! (`fake-key-1`), never real credentials.

mod tests {
    // Test fixtures are routinely built by starting from `Default::default()` and
    // filling in the fields a case exercises; that readability is worth more than
    // the struct-update rewrite the pedantic lint would demand.
    #![allow(clippy::field_reassign_with_default)]

    use crate::config_transfer::*;
    use crate::provider::{self, ModelAssignment, ModelRoute, SettingsFile, KEY_ENV};
    use crate::provider_metrics::QuotaSetting;
    use crate::secrets::{AuthFile, DeviceEndpoints, ProviderSecret};
    use std::path::Path;

    /// The decisions model the tool picker and decision roles must use.
    const JEVE: &str = "typesafe/jev-1.13";

    /// A complete, valid portable profile document. Each rejection test starts
    /// from this and edits exactly one thing.
    fn baseline_https() -> &'static str {
        r#"{
  "schema_version": 1,
  "exported_at": "2026-10-09T00:00:00Z",
  "providers": [
    {
      "id": "openrouter",
      "kind": "openrouter",
      "base_url": "https://openrouter.ai/api/v1",
      "default_model": "openai/gpt-4o-mini",
      "credential": {"source": "inline", "api_key": "fake-key-1", "name": ""},
      "quota_group_id": "openrouter"
    }
  ],
  "tool_credentials": [
    {"provider": "firecrawl", "primary": {"source": "inline", "api_key": "fake-key-fc", "name": ""}, "fallback": null}
  ],
  "model_roles": {
    "recon": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "synthesis": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "tool_picker": {"primary": {"provider_id": "openrouter", "model": "typesafe/jev-1.13"}, "fallbacks": []},
    "classifier": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "summarization": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "evidence_curator": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "entity_resolver": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "claim_assessor": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "investigation_controller": {"primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"}, "fallbacks": []},
    "decision_model": {"primary": {"provider_id": "openrouter", "model": "typesafe/jev-1.13"}, "fallbacks": []},
    "decision_fallback": null
  },
  "rate_limits": [
    {"quota_group_id": "openrouter", "scope": "*", "concurrency": 1, "source": "unset", "verified_at": ""}
  ]
}"#
    }

    /// Parse the baseline into a mutable JSON value for a single structural edit.
    fn doc() -> serde_json::Value {
        serde_json::from_str(baseline_https()).expect("the baseline is valid JSON")
    }

    /// Render an edited value back into text for `parse_document`.
    fn render(v: &serde_json::Value) -> String {
        serde_json::to_string(v).unwrap()
    }

    /// Build a role assignment with an empty account and ordered fallbacks.
    fn assignment(provider: &str, model: &str, fallbacks: &[(&str, &str)]) -> ModelAssignment {
        ModelAssignment {
            provider: provider.into(),
            model: model.into(),
            account: String::new(),
            fallbacks: fallbacks
                .iter()
                .map(|(p, m)| ModelRoute {
                    provider: (*p).to_string(),
                    model: (*m).to_string(),
                    account: String::new(),
                })
                .collect(),
        }
    }

    /// A default quota entry; `verified_*` stay `None`, local override `None`.
    fn rate_limit_entry(group: &str) -> QuotaSetting {
        QuotaSetting {
            quota_group_id: group.into(),
            scope: "*".into(),
            concurrency: 1,
            source: "unset".into(),
            verified_at: String::new(),
            ..Default::default()
        }
    }

    /// Look up a rate-limit entry by `(quota_group_id, scope)`.
    fn rate_limit<'a>(
        quotas: &'a QuotaSettingsFile,
        group: &str,
        scope: &str,
    ) -> Option<&'a QuotaSetting> {
        quotas
            .settings
            .iter()
            .find(|s| s.quota_group_id == group && s.scope == scope)
    }

    /// The exported entry for a provider id.
    fn entry_for<'a>(doc: &'a ProfileConfig, id: &str) -> &'a ProviderEntry {
        doc.providers
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("provider {id} was not exported"))
    }

    /// Auth with a single inline-key account for `kind`.
    fn inline_account(kind: &str, key: &str) -> AuthFile {
        let mut auth = AuthFile::default();
        let mut secret = provider::account_secret(&auth, kind);
        secret.api_key = Some(key.into());
        auth.set_account(secret);
        auth
    }

    /// Assert the directory holds no `*.tmp` or `*.staged` leftovers.
    fn assert_no_staged_files(dir: &Path) {
        let leftovers: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp") || name.ends_with(".staged"))
            .collect();
        assert!(leftovers.is_empty(), "staged leftovers: {leftovers:?}");
    }

    // ---- Round trip / export ---------------------------------------------

    #[test]
    fn export_document_reports_every_configured_account_and_role() {
        let mut auth = AuthFile::default();
        for (kind, key) in [("openrouter", "fake-key-or"), ("google", "fake-key-go")] {
            let mut secret = provider::account_secret(&auth, kind);
            secret.api_key = Some(key.into());
            auth.set_account(secret);
        }
        let mut settings = SettingsFile::default();
        settings.defaults.recon = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.tool_picker = assignment("openrouter", JEVE, &[]);
        let quotas = QuotaSettingsFile {
            settings: vec![rate_limit_entry("openrouter"), rate_limit_entry("google")],
        };

        let doc = export_document(&settings, &auth, &quotas);

        assert_eq!(doc.schema_version, SCHEMA_VERSION);
        assert!(doc.exported_at.is_some(), "export stamps RFC3339");
        assert_eq!(doc.providers.len(), 2, "one entry per configured account");
        assert_eq!(
            provider::effective_kind(&auth.account("openrouter").unwrap()),
            "openrouter"
        );

        let or = entry_for(&doc, "openrouter");
        assert_eq!(or.kind, "openrouter");
        assert_eq!(or.base_url, "https://openrouter.ai/api/v1");
        assert!(matches!(or.credential.source, CredentialSource::Inline));
        assert_eq!(or.credential.api_key, "fake-key-or");
        assert_eq!(or.quota_group_id, "openrouter");

        let gg = entry_for(&doc, "google");
        assert_eq!(gg.kind, "google");
        assert!(matches!(gg.credential.source, CredentialSource::Inline));
        assert_eq!(gg.credential.api_key, "fake-key-go");

        // Roles export verbatim.
        assert_eq!(
            doc.model_roles
                .recon
                .primary
                .as_ref()
                .map(|r| (&r.provider_id, &r.model)),
            Some((&"openrouter".to_string(), &"openai/gpt-4o-mini".to_string()))
        );
        assert_eq!(
            doc.model_roles
                .tool_picker
                .primary
                .as_ref()
                .map(|r| r.model.as_str()),
            Some(JEVE)
        );
        // rate_limits copies the supplied settings verbatim.
        assert_eq!(doc.rate_limits, quotas.settings);
    }

    #[test]
    fn export_document_enumerates_the_nine_keyed_tool_providers() {
        let settings = SettingsFile::default();
        let auth = AuthFile::default();
        let quotas = QuotaSettingsFile { settings: vec![] };

        let doc = export_document(&settings, &auth, &quotas);

        assert_eq!(doc.tool_credentials.len(), KEY_ENV.len());
        for (entry, (id, _)) in doc.tool_credentials.iter().zip(KEY_ENV.iter()) {
            assert_eq!(&entry.provider, id, "tool credentials follow KEY_ENV order");
        }
        assert!(
            doc.tool_credentials.iter().all(|e| e.provider != "holehe"),
            "holehe is keyless and must never appear"
        );
    }

    #[test]
    fn export_keeps_env_only_credentials_as_references() {
        // The name a keyed tool provider reads is fixed by KEY_ENV, so save and
        // restore the existing value rather than leaving an env change behind.
        const VAR: &str = "CURRENTS_API_KEY";
        let previous = std::env::var(VAR).ok();
        std::env::set_var(VAR, "currents-env-only-value-7q3");

        let settings = SettingsFile::default(); // no saved key -> env reference
        let auth = AuthFile::default();
        let quotas = QuotaSettingsFile { settings: vec![] };

        let doc = export_document(&settings, &auth, &quotas);

        let entry = doc
            .tool_credentials
            .iter()
            .find(|entry| entry.provider == "currents")
            .expect("currents is a keyed tool provider");
        let primary = match &entry.primary {
            Supplied::Value(credential) => credential,
            other => panic!("expected an env reference, found {other:?}"),
        };
        assert!(matches!(primary.source, CredentialSource::Env));
        assert_eq!(primary.name, "CURRENTS_API_KEY");
        assert_eq!(
            primary.api_key, "",
            "the env value is never inlined into the document"
        );
        // A second account that is neither saved nor in the environment is
        // exported as an explicit keyless slot, not omitted.
        match &entry.fallback {
            Supplied::Value(credential) => {
                assert!(matches!(credential.source, CredentialSource::None));
            }
            other => panic!("expected an explicit keyless fallback, found {other:?}"),
        }

        match previous {
            Some(old) => std::env::set_var(VAR, old),
            None => std::env::remove_var(VAR),
        }
    }

    #[test]
    fn export_preserves_intentional_inheritance() {
        let mut settings = SettingsFile::default();
        settings.defaults.recon = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        // summarization and evidence_curator stay empty (they inherit).

        let doc = export_document(
            &settings,
            &AuthFile::default(),
            &QuotaSettingsFile { settings: vec![] },
        );

        assert!(
            doc.model_roles.recon.primary.is_some(),
            "a configured role exports a route"
        );
        assert!(
            doc.model_roles.summarization.primary.is_none(),
            "an empty summarization stays inheriting, never flattened to a resolved default"
        );
        assert!(
            doc.model_roles.evidence_curator.primary.is_none(),
            "an empty evidence_curator stays inheriting"
        );
    }

    #[test]
    fn every_role_and_fallback_survives_a_round_trip() {
        let mut settings = SettingsFile::default();
        settings.defaults.recon = assignment(
            "openrouter",
            "openai/gpt-4o-mini",
            &[("google", "openai/gpt-4.1")],
        );
        settings.defaults.synthesis = assignment("openrouter", "openai/gpt-4o", &[]);
        settings.defaults.tool_picker = assignment(
            "openrouter",
            JEVE,
            &[("openrouter", "~typesafe/jev-latest")],
        );
        settings.defaults.classifier = assignment("google", "gemini-2.5-flash", &[]);
        settings.defaults.summarization = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.evidence_curator = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.entity_resolver = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.claim_assessor = assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.investigation_controller =
            assignment("openrouter", "openai/gpt-4o-mini", &[]);
        settings.defaults.decision_model = assignment("openrouter", JEVE, &[]);
        settings.defaults.decision_fallback = Some(assignment(
            "openrouter",
            JEVE,
            &[("openrouter", "~typesafe/jev-latest")],
        ));

        let auth = inline_account("openrouter", "fake-key-1");
        let quotas = QuotaSettingsFile {
            settings: vec![rate_limit_entry("openrouter")],
        };
        let before = toml::to_string_pretty(&settings.defaults).unwrap();

        let doc = export_document(&settings, &auth, &quotas);
        let text = serialize_document(&doc).unwrap();
        let plan = parse_document(&text).expect("a full export re-imports cleanly");

        let mut restored = SettingsFile::default();
        let mut restored_auth = AuthFile::default();
        let mut restored_quotas = QuotaSettingsFile { settings: vec![] };
        apply_import(
            &plan,
            &mut restored,
            &mut restored_auth,
            &mut restored_quotas,
        )
        .unwrap();

        let after = toml::to_string_pretty(&restored.defaults).unwrap();
        assert_eq!(
            before, after,
            "every role and ordered fallback survives verbatim"
        );
    }

    #[test]
    fn decision_fallback_round_trips_and_clears_on_null() {
        // Part A: a configured decision fallback round-trips.
        let mut settings = SettingsFile::default();
        settings.defaults.tool_picker = assignment("openrouter", JEVE, &[]);
        settings.defaults.decision_model = assignment("openrouter", JEVE, &[]);
        settings.defaults.decision_fallback = Some(assignment(
            "openrouter",
            JEVE,
            &[("openrouter", "~typesafe/jev-latest")],
        ));
        let auth = inline_account("openrouter", "fake-key-1");
        let quotas = QuotaSettingsFile {
            settings: vec![rate_limit_entry("openrouter")],
        };

        let text = serialize_document(&export_document(&settings, &auth, &quotas)).unwrap();
        let plan = parse_document(&text).unwrap();
        let mut restored = SettingsFile::default();
        let mut auth2 = AuthFile::default();
        let mut quotas2 = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut restored, &mut auth2, &mut quotas2).unwrap();

        let fallback = restored
            .defaults
            .decision_fallback
            .expect("decision fallback survives");
        assert_eq!(fallback.provider, "openrouter");
        assert_eq!(fallback.model, JEVE);
        assert_eq!(fallback.fallbacks.len(), 1);

        // Part B: an explicit null clears an existing decision fallback.
        let plan = parse_document(baseline_https())
            .expect("baseline with null decision_fallback is valid");
        let mut target = SettingsFile::default();
        target.defaults.decision_fallback = Some(assignment("openrouter", JEVE, &[]));
        let mut auth3 = AuthFile::default();
        let mut quotas3 = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut target, &mut auth3, &mut quotas3).unwrap();
        assert!(
            target.defaults.decision_fallback.is_none(),
            "decision_fallback: null clears it"
        );
    }

    // ---- Validation rejections -------------------------------------------

    #[test]
    fn rejects_a_document_larger_than_one_mebibyte() {
        let mut v = doc();
        v["exported_at"] = serde_json::json!("x".repeat(MAX_DOCUMENT_BYTES + 1024));
        let text = render(&v);
        assert!(
            text.len() > MAX_DOCUMENT_BYTES,
            "the document must exceed the cap"
        );
        assert!(
            parse_document(&text).is_err(),
            "an oversized document is refused up front"
        );
    }

    #[test]
    fn rejects_a_duplicate_object_key_at_the_root_and_when_nested() {
        // JSON literal surgery (not a JSON value) is required: a value would drop the
        // duplicate, so the raw text must carry two identical keys.
        let root = baseline_https().replacen(
            "\"schema_version\": 1,",
            "\"schema_version\": 1, \"schema_version\": 1,",
            1,
        );
        assert!(
            parse_document(&root).is_err(),
            "a duplicate key at the root object must be rejected"
        );

        let nested = baseline_https().replacen(
            "\"concurrency\": 1,",
            "\"concurrency\": 1, \"concurrency\": 1,",
            1,
        );
        assert!(
            parse_document(&nested).is_err(),
            "a duplicate key nested in rate_limits must be rejected"
        );
    }

    #[test]
    fn rejects_unknown_fields_with_a_pointer() {
        let mut v = doc();
        v["providers"][0]["knd"] = serde_json::json!("openrouter"); // typo for `kind`
        let err = parse_document(&render(&v)).unwrap_err();
        assert!(
            err.to_string().contains("/providers/0/knd"),
            "the error names the offending JSON pointer: {err}"
        );
    }

    #[test]
    fn rejects_a_missing_required_role_key() {
        let mut v = doc();
        assert!(
            v["model_roles"]
                .as_object_mut()
                .unwrap()
                .remove("recon")
                .is_some(),
            "recon is present in the baseline"
        );
        assert!(
            parse_document(&render(&v)).is_err(),
            "a missing required role key is a blocking error"
        );
    }

    #[test]
    fn rejects_a_quota_group_reference_to_a_missing_group() {
        let mut v = doc();
        v["providers"][0]["quota_group_id"] = serde_json::json!("missing-group");
        assert!(
            parse_document(&render(&v)).is_err(),
            "a provider quota_group must have a matching rate_limits entry"
        );
    }

    #[test]
    fn rejects_a_duplicate_quota_group_and_scope() {
        let mut v = doc();
        v["rate_limits"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "quota_group_id": "openrouter",
                "scope": "*",
                "concurrency": 1,
                "source": "unset",
                "verified_at": ""
            }));
        assert!(
            parse_document(&render(&v)).is_err(),
            "a duplicate (quota_group_id, scope) must be rejected"
        );
    }

    #[test]
    fn rejects_nonpositive_verified_limits_but_allows_a_zero_local_rpm() {
        let mut v = doc();
        v["rate_limits"][0]["verified_rpm"] = serde_json::json!(0);
        assert!(
            parse_document(&render(&v)).is_err(),
            "verified_rpm == 0 is rejected"
        );

        let mut v = doc();
        v["rate_limits"][0]["verified_tpm"] = serde_json::json!(0);
        assert!(
            parse_document(&render(&v)).is_err(),
            "verified_tpm == 0 is rejected"
        );

        let mut v = doc();
        v["rate_limits"][0]["verified_rpd"] = serde_json::json!(0);
        assert!(
            parse_document(&render(&v)).is_err(),
            "verified_rpd == 0 is rejected"
        );

        let mut v = doc();
        v["rate_limits"][0]["local_rpm"] = serde_json::json!(0);
        assert!(
            parse_document(&render(&v)).is_ok(),
            "an explicit zero local_rpm is allowed"
        );
    }

    #[test]
    fn rejects_an_out_of_range_concurrency() {
        let mut v = doc();
        v["rate_limits"][0]["concurrency"] = serde_json::json!(0);
        assert!(
            parse_document(&render(&v)).is_err(),
            "concurrency == 0 is rejected"
        );

        let mut v = doc();
        v["rate_limits"][0]["concurrency"] = serde_json::json!(65);
        assert!(
            parse_document(&render(&v)).is_err(),
            "concurrency > 64 is rejected"
        );

        let mut v = doc();
        v["rate_limits"][0]["concurrency"] = serde_json::json!(1);
        assert!(
            parse_document(&render(&v)).is_ok(),
            "concurrency == 1 is accepted"
        );

        let mut v = doc();
        v["rate_limits"][0]["concurrency"] = serde_json::json!(64);
        assert!(
            parse_document(&render(&v)).is_ok(),
            "concurrency == 64 is accepted"
        );
    }

    #[test]
    fn rejects_an_unknown_provider_kind_and_a_plain_http_endpoint() {
        let mut v = doc();
        v["providers"][0]["kind"] = serde_json::json!("mistral");
        assert!(
            parse_document(&render(&v)).is_err(),
            "a kind outside the schema enum is rejected"
        );

        let mut v = doc();
        v["providers"][0]["base_url"] = serde_json::json!("http://openrouter.ai/api");
        assert!(
            parse_document(&render(&v)).is_err(),
            "a non-https base_url is rejected when the kind is not local"
        );

        // A local endpoint is allowed to use a plain http scheme.
        let mut v = doc();
        v["providers"][0]["kind"] = serde_json::json!("local");
        v["providers"][0]["base_url"] = serde_json::json!("http://localhost:11434/v1");
        assert!(
            parse_document(&render(&v)).is_ok(),
            "a local provider may use a plain http endpoint"
        );
    }

    #[test]
    fn rejects_a_key_on_a_subscription_route() {
        let mut v = doc();
        v["providers"][0]["kind"] = serde_json::json!("subscription");
        // The baseline credential is inline; a subscription must be `none`.
        let err = parse_document(&render(&v)).unwrap_err();
        assert!(
            err.to_string().contains("/providers/0/credential"),
            "the error names the credential pointer: {err}"
        );

        // A subscription route with a keyless credential and a grok endpoint is accepted.
        let mut v = doc();
        v["providers"][0]["kind"] = serde_json::json!("subscription");
        v["providers"][0]["base_url"] = serde_json::json!("https://grok.com");
        v["providers"][0]["credential"] =
            serde_json::json!({"source": "none", "api_key": "", "name": ""});
        assert!(
            parse_document(&render(&v)).is_ok(),
            "a subscription route with a keyless credential is accepted"
        );
    }

    #[test]
    fn rejects_a_decisions_model_on_a_general_role_and_a_general_model_on_a_decision_role() {
        // A decisions (Jev) model on a general role is rejected, and the message
        // names the offending role.
        let mut v = doc();
        v["model_roles"]["recon"]["primary"]["model"] = serde_json::json!(JEVE);
        let err = parse_document(&render(&v)).unwrap_err();
        assert!(
            err.to_string().contains("recon"),
            "the error names the general role: {err}"
        );

        // A general (non-Jev) model on each decision role is rejected.
        for role in ["tool_picker", "decision_model"] {
            let mut v = doc();
            v["model_roles"][role]["primary"]["model"] = serde_json::json!("openai/gpt-4o-mini");
            assert!(
                parse_document(&render(&v)).is_err(),
                "a general model on {role} is rejected"
            );
        }

        let mut v = doc();
        v["model_roles"]["decision_fallback"] = serde_json::json!({
            "primary": {"provider_id": "openrouter", "model": "openai/gpt-4o-mini"},
            "fallbacks": []
        });
        assert!(
            parse_document(&render(&v)).is_err(),
            "a general model on decision_fallback is rejected"
        );
    }

    #[test]
    fn rejects_a_holehe_tool_credential() {
        let mut v = doc();
        v["tool_credentials"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "provider": "holehe",
                "primary": {"source": "none", "api_key": "", "name": ""},
                "fallback": {"source": "none", "api_key": "", "name": ""}
            }));
        let err = parse_document(&render(&v)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("keyed") || KEY_ENV.iter().any(|(id, _)| msg.contains(id)),
            "the message names the allowed keyed providers: {msg}"
        );

        // The nine real keyed providers are accepted.
        let mut v = doc();
        let entries: Vec<serde_json::Value> = KEY_ENV
            .iter()
            .map(|(id, _)| {
                serde_json::json!({
                    "provider": id,
                    "primary": {"source": "none", "api_key": "", "name": ""},
                    "fallback": {"source": "none", "api_key": "", "name": ""}
                })
            })
            .collect();
        v["tool_credentials"] = serde_json::Value::Array(entries);
        assert!(
            parse_document(&render(&v)).is_ok(),
            "each keyed tool provider is accepted"
        );
    }

    #[test]
    fn a_syntax_error_reports_line_and_column() {
        // Cut the document mid-value so the parser reports a 1-based location.
        let truncated = &baseline_https()[..40];
        let err = parse_document(truncated).unwrap_err();
        let text = err.to_string();
        assert!(
            regex::Regex::new(r"line \d+").unwrap().is_match(&text),
            "the syntax error reports a 1-based line: {text}"
        );
        assert!(
            regex::Regex::new(r"column \d+").unwrap().is_match(&text),
            "the syntax error reports a 1-based column: {text}"
        );
    }

    #[test]
    fn an_error_message_never_echoes_a_key() {
        let mut v = doc();
        v["providers"][0]["credential"]["api_key"] = serde_json::json!("sk-secret-value-123");
        v["providers"][0]["unexpected"] = serde_json::json!(true); // unknown field beside the key
        let err = parse_document(&render(&v)).unwrap_err();
        assert!(
            !format!("{err}").contains("sk-secret-value-123"),
            "a key value must never appear in an error message: {err}"
        );
    }

    // ---- Warnings + merge -------------------------------------------------

    #[test]
    fn an_unresolved_env_reference_warns_instead_of_importing_an_empty_key() {
        const VAR: &str = "FIRECRAWL_API_KEY";
        let previous = std::env::var(VAR).ok();
        // Reference the variable, then remove it before parsing so the reference
        // cannot resolve at import time.
        std::env::set_var(VAR, "should-never-settle-into-the-document");
        std::env::remove_var(VAR);

        let mut settings = SettingsFile {
            firecrawl_api_key: "local-firecrawl".into(),
            ..Default::default()
        };

        let mut v = doc();
        v["tool_credentials"][0]["primary"] =
            serde_json::json!({"source": "env", "api_key": "", "name": VAR});
        let plan = parse_document(&render(&v))
            .expect("an unresolved env reference warns, it does not block");
        assert!(
            !plan.warnings.is_empty(),
            "an unresolved env reference produces a warning"
        );
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.message.contains(VAR) || w.pointer.contains("primary")),
            "the warning names the unresolved variable: {:?}",
            plan.warnings
                .iter()
                .map(|w| w.message.clone())
                .collect::<Vec<_>>()
        );

        let mut auth = AuthFile::default();
        let mut quotas = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();
        assert_eq!(
            settings.saved_key("firecrawl"),
            "local-firecrawl",
            "an unresolved env reference is not imported as an empty key"
        );

        match previous {
            Some(old) => std::env::set_var(VAR, old),
            None => std::env::remove_var(VAR),
        }
    }

    #[test]
    fn merge_replaces_supplied_values_and_preserves_unrelated_settings() {
        let mut settings = SettingsFile {
            firecrawl_api_key: "local-firecrawl".into(),
            newsapi_api_key: "local-newsapi".into(),
            osint_user_agent: "local-agent".into(),
            tui_recon_context: Some(true),
            model: "local-model".into(),
            writer_model: "local-writer".into(),
            ..Default::default()
        };
        settings.recon_limits.max_rounds = 7;
        settings.recon_limits.hunter_credits = 12345;
        settings.defaults.recon = assignment("google", "old-recon", &[]);

        // A document that supplies only a firecrawl primary key and its roles.
        let plan = parse_document(baseline_https()).expect("the baseline is valid");
        let mut auth = AuthFile::default();
        let mut quotas = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();

        // Supplied values are applied.
        assert_eq!(settings.saved_key("firecrawl"), "fake-key-fc");
        assert_eq!(
            settings.defaults.recon.provider, "openrouter",
            "the supplied recon route is applied"
        );
        assert_eq!(settings.defaults.recon.model, "openai/gpt-4o-mini");

        // Unrelated settings are untouched.
        assert_eq!(
            settings.recon_limits.max_rounds, 7,
            "recon_limits are preserved"
        );
        assert_eq!(settings.recon_limits.hunter_credits, 12345);
        assert_eq!(
            settings.osint_user_agent, "local-agent",
            "the OSINT user agent is preserved"
        );
        assert_eq!(
            settings.tui_recon_context,
            Some(true),
            "the TUI flag is preserved"
        );
        assert_eq!(
            settings.newsapi_api_key, "local-newsapi",
            "an unmentioned saved key is preserved"
        );
        assert_eq!(settings.model, "local-model");
        assert_eq!(settings.writer_model, "local-writer");
    }

    #[test]
    fn explicit_null_clears_a_role_to_documented_inheritance() {
        let mut settings = SettingsFile::default();
        settings.defaults.evidence_curator =
            assignment("openrouter", "old-curator", &[("google", "curator-backup")]);

        let mut v = doc();
        v["model_roles"]["evidence_curator"]["primary"] = serde_json::json!(null);
        let plan = parse_document(&render(&v)).expect("valid");
        let mut auth = AuthFile::default();
        let mut quotas = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();

        let curator = &settings.defaults.evidence_curator;
        assert_eq!(curator.provider, "", "a null primary empties the provider");
        assert_eq!(curator.model, "", "a null primary empties the model");
        assert_eq!(curator.account, "", "a null primary empties the account");
        assert!(
            curator.fallbacks.is_empty(),
            "a null primary also clears the fallbacks"
        );
    }

    #[test]
    fn tool_credential_none_clears_and_null_preserves() {
        // (1) A `none` credential clears the local saved key.
        {
            let mut settings = SettingsFile {
                firecrawl_api_key: "local-primary".into(),
                ..Default::default()
            };
            let mut v = doc();
            v["tool_credentials"][0]["primary"] =
                serde_json::json!({"source": "none", "api_key": "", "name": ""});
            let plan = parse_document(&render(&v)).unwrap();
            let mut auth = AuthFile::default();
            let mut quotas = QuotaSettingsFile { settings: vec![] };
            apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();
            assert_eq!(
                settings.saved_key("firecrawl"),
                "",
                "a none credential clears the saved key"
            );
        }

        // (2) An explicit null slot clears the local value; an absent slot preserves it.
        {
            let mut settings = SettingsFile {
                firecrawl_api_key: "local-primary".into(),
                firecrawl_api_key_fallback: "local-fallback".into(),
                ..Default::default()
            };
            let mut v = doc();
            assert!(
                v["tool_credentials"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("primary")
                    .is_some(),
                "the baseline primary slot is present to remove"
            );
            // fallback stays an explicit null in the baseline -> Removed.
            let plan = parse_document(&render(&v)).unwrap();
            let mut auth = AuthFile::default();
            let mut quotas = QuotaSettingsFile { settings: vec![] };
            apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();
            assert_eq!(
                settings.saved_key("firecrawl"),
                "local-primary",
                "an absent slot preserves the local value"
            );
            assert_eq!(
                settings.saved_fallback_key("firecrawl"),
                "",
                "an explicit null clears the local value"
            );
        }

        // (3) An omitted fallback key leaves the local fallback untouched.
        {
            let mut settings = SettingsFile {
                firecrawl_api_key_fallback: "local-fallback".into(),
                ..Default::default()
            };
            let mut v = doc();
            assert!(
                v["tool_credentials"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("fallback")
                    .is_some(),
                "the baseline fallback slot is present to remove"
            );
            let plan = parse_document(&render(&v)).unwrap();
            let mut auth = AuthFile::default();
            let mut quotas = QuotaSettingsFile { settings: vec![] };
            apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();
            assert_eq!(
                settings.saved_fallback_key("firecrawl"),
                "local-fallback",
                "an omitted key preserves the local value"
            );
            assert_eq!(
                settings.saved_key("firecrawl"),
                "fake-key-fc",
                "the inline primary is still applied"
            );
        }
    }

    #[test]
    fn rate_limits_merge_by_group_and_scope_and_keep_absent_groups() {
        let mut quotas = QuotaSettingsFile {
            settings: vec![
                QuotaSetting {
                    quota_group_id: "openrouter".into(),
                    scope: "*".into(),
                    local_rpm: Some(10),
                    ..Default::default()
                },
                QuotaSetting {
                    quota_group_id: "firecrawl".into(),
                    scope: "*".into(),
                    local_rpm: Some(5),
                    ..Default::default()
                },
            ],
        };

        let mut v = doc();
        v["rate_limits"][0]["local_rpm"] = serde_json::json!(77); // change openrouter/*
        v["rate_limits"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "quota_group_id": "openrouter",
                "scope": "openai/gpt-4o-mini",
                "concurrency": 2,
                "source": "unset",
                "verified_at": ""
            }));
        v["rate_limits"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "quota_group_id": "gnews",
                "scope": "*",
                "concurrency": 1,
                "source": "unset",
                "verified_at": ""
            }));

        let plan = parse_document(&render(&v)).expect("valid");
        let mut settings = SettingsFile::default();
        let mut auth = AuthFile::default();
        apply_import(&plan, &mut settings, &mut auth, &mut quotas).unwrap();

        assert_eq!(
            quotas.settings.len(),
            4,
            "openrouter/*, openrouter/scope, firecrawl/*, and gnews/*"
        );
        assert_eq!(
            rate_limit(&quotas, "openrouter", "*").unwrap().local_rpm,
            Some(77),
            "a supplied entry replaces its matching target"
        );
        assert_eq!(
            rate_limit(&quotas, "firecrawl", "*").unwrap().local_rpm,
            Some(5),
            "a stored group absent from the document survived"
        );
        assert!(
            rate_limit(&quotas, "openrouter", "openai/gpt-4o-mini").is_some(),
            "a new scope entry is added"
        );
        assert!(
            rate_limit(&quotas, "gnews", "*").is_some(),
            "a brand new group is added"
        );
    }

    #[test]
    fn the_change_summary_is_redacted() {
        let mut settings = SettingsFile::default();
        settings.defaults.recon = assignment("google", "old-recon", &[]);
        let auth = inline_account("openrouter", "fake-old-key");
        let quotas = QuotaSettingsFile {
            settings: vec![rate_limit_entry("openrouter")],
        };

        // parse_document never computes changes; they are derived against live state.
        let plan = parse_document(baseline_https()).expect("the baseline is valid");
        assert!(
            plan.changes.is_empty(),
            "parse_document leaves changes to changes_against"
        );

        let changes = plan.changes_against(&settings, &auth, &quotas);
        assert!(
            !changes.is_empty(),
            "a differing role and provider key produce changes"
        );
        assert!(
            changes.iter().any(|c| c.area == "role"),
            "a role difference is reported"
        );
        for change in &changes {
            assert!(
                !change.summary.contains("fake-key-1") && !change.summary.contains("fake-old-key"),
                "a change summary must never echo a key: {}",
                change.summary
            );
            assert!(
                !change.id.contains("fake-key-1") && !change.id.contains("fake-old-key"),
                "a change id must never echo a key: {}",
                change.id
            );
        }
    }

    #[test]
    fn a_subscription_account_keeps_its_device_endpoints_through_an_import() {
        let secret = ProviderSecret {
            kind: "grok-subscription".into(),
            base_url: "https://grok.com".into(),
            model: "grok-4.6".into(),
            api_key: None,
            stt_model: Some("whisper-1".into()),
            device: Some(DeviceEndpoints {
                client_id: "cid".into(),
                device_auth_url: "https://auth.x.ai/device".into(),
                token_url: "https://auth.x.ai/token".into(),
                scope: "openid profile email".into(),
            }),
        };

        let mut source = AuthFile::default();
        source.set_account(secret.clone());
        let quotas = QuotaSettingsFile {
            settings: vec![
                rate_limit_entry("grok"),
                rate_limit_entry("subscription"),
                rate_limit_entry("grok-subscription"),
            ],
        };
        let text = serialize_document(&export_document(&SettingsFile::default(), &source, &quotas))
            .unwrap();
        let plan = parse_document(&text).expect("a subscription export re-imports cleanly");

        // The target is the same machine re-importing its profile: it already holds
        // the OAuth login, which the transfer document never carries.
        let mut target = AuthFile::default();
        target.set_account(secret);
        let mut settings = SettingsFile::default();
        let mut target_quotas = QuotaSettingsFile { settings: vec![] };
        apply_import(&plan, &mut settings, &mut target, &mut target_quotas).unwrap();

        let restored = target
            .accounts
            .values()
            .find(|s| s.device.as_ref().is_some_and(|d| d.client_id == "cid"))
            .expect("the device-endpoint account survives the import");
        assert_eq!(
            restored.device.as_ref().unwrap().client_id,
            "cid",
            "the device endpoints survive the import"
        );
        assert_eq!(
            restored.stt_model.as_deref(),
            Some("whisper-1"),
            "the stt model survives the import"
        );
        assert!(
            restored.api_key.is_none(),
            "a subscription keeps no saved key"
        );
    }

    #[test]
    fn write_export_is_deterministic_and_owner_only() {
        // The baseline carries a fixed `exported_at`, so the serialised bytes are
        // deterministic without any timestamp normalisation.
        let config = parse_document(baseline_https()).unwrap().config;

        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.json");
        let b = dir.path().join("b.json");
        write_export(&config, &a).unwrap();
        write_export(&config, &b).unwrap();

        assert_eq!(
            std::fs::read(&a).unwrap(),
            std::fs::read(&b).unwrap(),
            "the same document serialises to identical bytes"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&a).unwrap().permissions().mode() & 0o777,
                0o600,
                "the exported file is owner-only"
            );
        }
        assert_no_staged_files(dir.path());
    }

    #[test]
    fn a_failed_validation_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("profile.json");

        let mut v = doc();
        v["providers"][0]["kind"] = serde_json::json!("madeup"); // invalid document
        let outcome = parse_document(&render(&v));
        assert!(outcome.is_err(), "an invalid document is rejected up front");

        // A rejected import never reaches the writer, so no file lands at the
        // target and no staged sibling is left behind.
        if let Ok(plan) = outcome {
            write_export(&plan.config, &target).unwrap();
        }
        assert!(!target.exists(), "a failed validation writes no file");
        assert_no_staged_files(dir.path());
    }
}
