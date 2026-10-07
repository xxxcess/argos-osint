import re

with open("crates/argos-osint-bin/src/tui/app.rs", "r") as f:
    app_text = f.read()

def extract_enum(text, enum_name):
    match = re.search(f'pub enum {enum_name} {{(.*?)}}', text, re.DOTALL)
    if not match: return []
    items = []
    lines = match.group(1).split('\n')
    for line in lines:
        line = line.split('//')[0].strip()
        line = line.split('///')[0].strip()
        if not line: continue
        name = line.split('(')[0].split('{')[0].strip().rstrip(',')
        if name and not name.startswith('#'): items.append(name)
    return items

modules = extract_enum(app_text, "ModuleId")
overlays = extract_enum(app_text, "Overlay")
targets = extract_enum(app_text, "Target")
fields = extract_enum(app_text, "FieldId")

markdown = "# Argos UI Interaction Audit\n\n"
markdown += "## Audit Matrix\n\n"
markdown += "| Screen / Overlay | State Variant | Targets / Controls | Visual Order | Input Hints | Scroll / Overflow | Focused Style | Narrow Layout | Verification |\n"
markdown += "|---|---|---|---|---|---|---|---|---|\n"

for mod in modules:
    markdown += f"| {mod} | Default | | | | | | | Pending |\n"

for overlay in overlays:
    if overlay != "None":
        markdown += f"| Overlay: {overlay} | Default | | | | | | | Pending |\n"

for field in fields:
    markdown += f"| Field: {field} | Default | | | | | | | Pending |\n"

with open("docs/ui-interaction-audit.md", "w") as f:
    f.write(markdown)
