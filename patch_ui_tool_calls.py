import re

with open("crates/argos-osint-bin/src/tui/ui.rs", "r") as f:
    text = f.read()

old_code = """
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} · sec {}/{} · el {}/{} · tools {}/{}",
                    job.stage,
                    job.sections_done,
                    job.sections_total,
                    job.elements_done,
                    job.elements_total,
                    job.tool_calls_done,
                    job.tool_calls_allowance
                ),
                theme::dim(),
            )));
"""

new_code = """
            let tools_str = if job.tool_calls_done == -1 {
                "Tool usage unavailable".to_string()
            } else if job.tool_calls_done == 0 && (job.state == "completed" || job.state == "partial" || job.state == "failed") {
                "0 calls used · reused evidence".to_string()
            } else {
                format!("Tools: {} calls used · {} budget", job.tool_calls_done, job.tool_calls_allowance)
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {} · sec {}/{} · el {}/{} · {}",
                    job.stage,
                    job.sections_done,
                    job.sections_total,
                    job.elements_done,
                    job.elements_total,
                    tools_str
                ),
                theme::dim(),
            )));
"""

text = text.replace(old_code.strip('\n'), new_code.strip('\n'))

with open("crates/argos-osint-bin/src/tui/ui.rs", "w") as f:
    f.write(text)

