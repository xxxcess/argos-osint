import re

with open("crates/argos-osint-core/src/atlas_insights.rs", "r") as f:
    code = f.read()

# Remove the store variable from the top of extract
code = code.replace("    let store = crate::store::Store::open(&crate::paths::db_path())?;\n", "")

# Change completed_units loading to a block
completed_block = """
    let mut completed_ids = std::collections::HashSet::new();
    if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
        let completed_units = store.atlas_get_unit_manifests(run_id, 4).unwrap_or_default();
        for unit in &completed_units {
            if unit.terminal_reason.is_none() {
                completed_ids.insert(unit.unit_id.clone());
            }
        }
    }
"""

code = re.sub(r'    let completed_units = store\.atlas_get_unit_manifests\(run_id, 4\)\.unwrap_or_default\(\);\n    let mut completed_ids = std::collections::HashSet::new\(\);\n    for unit in &completed_units \{\n        if unit\.terminal_reason\.is_none\(\) \{\n            completed_ids\.insert\(unit\.unit_id\.clone\(\)\);\n        \}\n    \}', completed_block.strip(), code)


# Change save blocks
save_ok_old = """                let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {"""
save_ok_new = """                if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
                    let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {"""
code = code.replace(save_ok_old, save_ok_new)
code = code.replace("                    terminal_reason: None,\n                });\n            }", "                    terminal_reason: None,\n                });\n                }\n            }")

save_err_old = """                let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {"""
save_err_new = """                if let Ok(store) = crate::store::Store::open(&crate::paths::db_path()) {
                    let _ = store.atlas_save_unit_manifest(&crate::atlas_work::UnitManifest {"""
# already replaced by the above, wait let's just do regex

with open("crates/argos-osint-core/src/atlas_insights.rs", "w") as f:
    f.write(code)

