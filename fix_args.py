import os
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()

    # handle inline:
    content = content.replace(
        'store.update_report_job(&job.id, "queued", "planned", 0, 0, 0, "", "", "")?;',
        'store.update_report_job(&job.id, "queued", "planned", 0, 0, "", "")?;'
    )

    # handle multi-line block
    # We want to remove lines containing `job.tool_calls_done` (or `job.tool_calls_done + 1`)
    # and lines containing `&job.current_tool` or `"firecrawl_search"` right after it or `""` right after it.
    
    lines = content.split('\n')
    new_lines = []
    skip = False
    for line in lines:
        if 'job.tool_calls_done' in line and not 'job.tool_calls_done,' in line.replace('job.tool_calls_done', 'job.tool_calls_done,'): # roughly
            # we want to delete this line and the next line if it's the 6th and 7th argument
            if 'job.tool_calls_done' in line and ('update_report_job' not in line):
                continue # this just skips the line containing job.tool_calls_done
        if line.strip() == '&job.current_tool,' or line.strip() == '"firecrawl_search",':
            continue
        # Also there's cases where it's `""` instead of `&job.current_tool`
        
        new_lines.append(line)

    with open(filepath, 'w') as f:
        f.write('\n'.join(new_lines))

process_file('crates/argos-osint-core/src/intel_recon/jobs.rs')
process_file('crates/argos-osint-core/src/intel_recon/worker.rs')
