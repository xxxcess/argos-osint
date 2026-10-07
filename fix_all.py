import re

with open("crates/argos-osint-core/src/atlas_insights.rs", "r") as f:
    code = f.read()

# 1. Fix is_significant
code = re.sub(r'pub fn is_significant\(category: &str\) -> bool \{\n    category_tag\(category\) != "unk"\n\}',
              'pub fn is_significant(category: &str) -> bool {\n    let tag = category_tag(category);\n    tag != "unk" && tag != "failed" && tag != "ambiguous" && tag != "out_of_scope"\n}', code)

# 2. Fix parse_claims
code = code.replace("""    // Empty object, missing claims, or a soft "nothing found" reply.
    if value.get("claims").is_none() {
        return Ok(Vec::new());
    }""", "")

code = code.replace("""    if trimmed.is_empty() {
        return Ok(serde_json::json!({"claims": []}));
    }""", """    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("Empty extraction output"));
    }""")

code = code.replace("""    let Some(start) = trimmed.find(['{', '[']) else {
        // Prose with no JSON object — treat as no claims for this packet.
        return Ok(serde_json::json!({"claims": []}));
    };""", """    let Some(start) = trimmed.find(['{', '[']) else {
        return Err(anyhow::anyhow!("No JSON object found in extraction output"));
    };""")

code = code.replace("""    let Some(end) = trimmed.rfind(close) else {
        return Ok(serde_json::json!({"claims": []}));
    };
    if end < start {
        return Ok(serde_json::json!({"claims": []}));
    }
    match serde_json::from_str(&trimmed[start..=end]) {
        Ok(value) => Ok(value),
        Err(_) => Ok(serde_json::json!({"claims": []})),
    }""", """    let Some(end) = trimmed.rfind(close) else {
        return Err(anyhow::anyhow!("Incomplete JSON object in extraction output"));
    };
    if end < start {
        return Err(anyhow::anyhow!("Malformed JSON structure in extraction output"));
    }
    match serde_json::from_str(&trimmed[start..=end]) {
        Ok(value) => Ok(value),
        Err(e) => Err(anyhow::anyhow!("Failed to parse JSON extraction output: {}", e)),
    }""")


# 3. Add run_id parameter to extract
code = code.replace("""pub async fn extract(
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,""", """pub async fn extract(
    run_id: &str,
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,""")

# 4. Modify extract loop for packet receipts without holding Store across await
extract_loop_old = """    for packet in significant.chunks(PACKET_LIMIT) {
        match ask_claims(synthesis, &lead_prompt(packet.len()), &packet_json(packet)?).await {
            Ok(raw) => {
                let (kept, dropped) = accept_claims(packet, &raw, AcceptMode::Lead);
                lead_dropped += dropped;
                lead.extend(kept);
            }
            Err(err) => {
                extract_error = Some(err.to_string());
            }
        }
        done += 1;
        progress(done, total);
    }"""

extract_loop_new = """
    let mut completed_ids = std::collections::HashSet::new();
    if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
        let completed_units = store.atlas_get_unit_manifests(run_id, 4).unwrap_or_default();
        for unit in &completed_units {
            if unit.terminal_reason.is_none() {
                completed_ids.insert(unit.unit_id.clone());
            }
        }
    }

    for (i, packet) in significant.chunks(PACKET_LIMIT).enumerate() {
        let unit_id = format!("extract-lead-{i}");
        if completed_ids.contains(&unit_id) {
            done += 1;
            progress(done, total);
            continue;
        }

        match ask_claims(synthesis, &lead_prompt(packet.len()), &packet_json(packet)?).await {
            Ok(raw) => {
                let (kept, dropped) = accept_claims(packet, &raw, AcceptMode::Lead);
                lead_dropped += dropped;
                lead.extend(kept);
                if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
                    let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {
                        run_id: run_id.to_string(),
                        unit_id,
                        stage: 4,
                        input_ids: packet.iter().map(|a| a.id.clone()).collect(),
                        input_rev: String::new(),
                        contract_version: "1".into(),
                        dependency_ids: Vec::new(),
                        is_required: true,
                        output_refs: Vec::new(),
                        effective_model: synthesis.model.clone(),
                        attempt_history: vec![chrono::Utc::now().to_rfc3339()],
                        next_eligible_at: None,
                        terminal_reason: None,
                    });
                }
            }
            Err(err) => {
                extract_error = Some(err.to_string());
                if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
                    let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {
                        run_id: run_id.to_string(),
                        unit_id,
                        stage: 4,
                        input_ids: packet.iter().map(|a| a.id.clone()).collect(),
                        input_rev: String::new(),
                        contract_version: "1".into(),
                        dependency_ids: Vec::new(),
                        is_required: true,
                        output_refs: Vec::new(),
                        effective_model: synthesis.model.clone(),
                        attempt_history: vec![chrono::Utc::now().to_rfc3339()],
                        next_eligible_at: None,
                        terminal_reason: Some(err.to_string()),
                    });
                }
            }
        }
        done += 1;
        progress(done, total);
    }
"""

code = code.replace(extract_loop_old, extract_loop_new.strip())

# 5. Fix tests in atlas_insights.rs
code = code.replace("""    fn claim_json_tolerates_empty_and_alternate_shapes() {
        assert!(parse_claims("").unwrap().is_empty());
        assert!(parse_claims("{}").unwrap().is_empty());
        assert!(parse_claims(r#"{"claims":null}"#).unwrap().is_empty());
        assert!(parse_claims("no claims this time").unwrap().is_empty());""", """    fn claim_json_tolerates_empty_and_alternate_shapes() {
        assert!(parse_claims("").is_err());
        assert!(parse_claims("{}").is_err());
        assert!(parse_claims(r#"{"claims":null}"#).unwrap().is_empty());
        assert!(parse_claims("no claims this time").is_err());
        assert!(parse_claims(r#"{"claims":[]}"#).unwrap().is_empty());
        assert!(parse_claims(r#"[]"#).unwrap().is_empty());""")


with open("crates/argos-osint-core/src/atlas_insights.rs", "w") as f:
    f.write(code)
