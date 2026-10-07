cat << 'INNER_EOF' > script.py
import sys
content = open("crates/argos-osint-core/src/atlas_work.rs").read()
old = """pub fn reduce_cycle_outcome(_manifests: &[UnitManifest]) -> CycleOutcome {
    // Cycle reduction logic stub
    CycleOutcome::Pending
}"""
new = """pub fn reduce_cycle_outcome(manifests: &[UnitManifest]) -> CycleOutcome {
    if manifests.is_empty() {
        return CycleOutcome::Pending;
    }

    let mut has_pending = false;
    let mut has_failed_required = false;
    let mut warnings = Vec::new();

    for m in manifests {
        if let Some(ref reason) = m.terminal_reason {
            if m.is_required {
                has_failed_required = true;
            } else {
                warnings.push(format!("{}: {}", m.unit_id, reason));
            }
        } else if m.next_eligible_at.is_some() || m.attempt_history.is_empty() {
            has_pending = true;
        }
    }

    if has_failed_required {
        return CycleOutcome::Failed;
    }

    if has_pending {
        return CycleOutcome::Pending;
    }

    if !warnings.is_empty() {
        return CycleOutcome::CompletedWithWarnings(warnings);
    }

    CycleOutcome::Completed
}"""
if old in content:
    open("crates/argos-osint-core/src/atlas_work.rs", "w").write(content.replace(old, new))
else:
    print("Old not found")
INNER_EOF
python3 script.py
