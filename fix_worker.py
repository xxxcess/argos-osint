import re

with open('crates/argos-osint-core/src/intel_recon/worker.rs', 'r') as f:
    content = f.read()

# We need to remove the 6th and 7th arguments to update_report_job
# update_report_job(id, state, stage, sections_done, elements_done, warning, error)
# Previous signature: (id, state, stage, sections_done, elements_done, tool_calls_done, current_tool, warning, error)
# It was called in worker.rs and jobs.rs

# Since it's easier to just do it via regex, let's fix it properly.
