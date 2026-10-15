use super::*;

fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    // Minimal structural check of the subset the schemas use.
    fn check(s: &Value, v: &Value, path: &str) -> Result<(), String> {
        match s["type"].as_str() {
            Some("object") => {
                let o = v.as_object().ok_or(format!("{path}: not object"))?;
                for r in s["required"].as_array().unwrap() {
                    if !o.contains_key(r.as_str().unwrap()) {
                        return Err(format!("{path}: missing {r}"));
                    }
                }
                for k in o.keys() {
                    let p = s["properties"].get(k).ok_or(format!("{path}: extra {k}"))?;
                    check(p, &o[k], &format!("{path}.{k}"))?;
                }
            }
            Some("array") => {
                let a = v.as_array().ok_or(format!("{path}: not array"))?;
                if let Some(min) = s["minItems"].as_u64() {
                    if (a.len() as u64) < min {
                        return Err(format!("{path}: too few"));
                    }
                }
                for x in a {
                    check(&s["items"], x, path)?;
                }
            }
            Some("string") => {
                let t = v.as_str().ok_or(format!("{path}: not string"))?;
                if let Some(e) = s["enum"].as_array() {
                    if !e.iter().any(|x| x == t) {
                        return Err(format!("{path}: {t} not offered"));
                    }
                }
            }
            Some("boolean") => {
                v.as_bool().ok_or(format!("{path}: not bool"))?;
            }
            _ => {}
        }
        Ok(())
    }
    check(schema, value, "$")
}

#[test]
fn schemas_are_strict_objects_with_offered_ids() {
    let claims = claims_schema(&["t1", "d1"]);
    assert_eq!(claims["additionalProperties"], false);
    let ok = json!({"claims":[{"id":"c1","text":"A said B","subject_id":"","predicate":"said","object":"B","attributed_to":"A","modality":"asserted","polarity":"positive","time":"","evidence_ids":["t1"]}],"limit_reached":false,"empty_reason":""});
    validate(&claims, &ok).unwrap();
    let mut bad = ok.clone();
    bad["claims"][0]["evidence_ids"] = json!(["b9"]);
    assert!(validate(&claims, &bad).unwrap_err().contains("not offered"));
    let mut extra = ok.clone();
    extra["claims"][0]["confidence"] = json!(0.9);
    assert!(validate(&claims, &extra).is_err());

    let inf = inferences_schema(&["c1"], &["b1", "b2"]);
    let ok = json!({"inferences":[{"id":"i1","text":"x","premise_claim_ids":["c1"],"body_evidence_ids":["b2"],"support":"conditional","why":"y","limitations":["z"],"counterevidence_ids":[]}],"empty_reason":""});
    validate(&inf, &ok).unwrap();
    let leads = leads_schema(&["r1"], &["i1"], &["r1.t1"]);
    let ok = json!({"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"w","verification_needed":"v"}],"empty_reason":""});
    validate(&leads, &ok).unwrap();
    let crate::provider_adapters::ResponseContract::GeneratedJson {
        schema: Some(selection),
        ..
    } = inference_selection_contract(&["i1".into(), "i2".into()])
    else {
        panic!("selection schema missing")
    };
    validate(&selection, &json!({"selected_ids":["i2"]})).unwrap();
    assert!(validate(&selection, &json!({"selected_ids":["invented"]})).is_err());
    assert!(validate(&selection, &json!({"selected_ids":[]})).is_err());
    assert!(validate(&selection, &json!({"selected_ids":["i1"],"inferences":[]})).is_err());
}

#[test]
fn limits_text_comes_from_the_validator_constants() {
    let t = inferences_limits();
    for n in [
        "at most 5 conclusions",
        "35 words",
        "45 words",
        "at most 2 limitations",
        "20 words",
        "25 words",
    ] {
        assert!(t.contains(n), "{n}");
    }
    assert!(claims_limits().contains("at most 6 claims"));
    assert!(leads_limits().contains("40 words"));
}

#[test]
fn contracts_compile_on_every_adapter_without_inventing_a_shape() {
    let c = claims_contract(&["t1"]);
    let crate::provider_adapters::ResponseContract::GeneratedJson {
        schema: Some(s), ..
    } = &c
    else {
        panic!()
    };
    assert_eq!(s, &claims_schema(&["t1"]));
}
