#!/usr/bin/env python3
"""Build a self-contained HTML docs site under docs/html/ for browser review.

Reads Markdown in docs/ plus the repo README and AGENTS.md. Diagrams stay in
docs/diagrams/. Open docs/index.html (or docs/html/index.html) in a browser.
"""

from __future__ import annotations

import html
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
OUT = DOCS / "html"
DIAGRAMS = DOCS / "diagrams"

PAGES = [
    ("index", "Overview", None),
    ("usage", "Usage", DOCS / "usage.md"),
    ("concepts", "Concepts", DOCS / "concepts.md"),
    ("architecture", "Architecture", DOCS / "architecture.md"),
    ("providers", "Providers", DOCS / "providers.md"),
    ("conventions", "Conventions", DOCS / "conventions.md"),
    ("diagrams", "Diagrams", DOCS / "diagrams.md"),
    ("gallery", "Gallery", None),
    ("investigation-harness", "Harness", DOCS / "investigation-harness.md"),
    ("jev-decision-gates", "Jev gates", DOCS / "jev-decision-gates.md"),
    ("opencode-v2", "OpenCode", DOCS / "opencode-v2.md"),
    ("ui-interaction-audit", "UI audit", DOCS / "ui-interaction-audit.md"),
    ("agents", "Agents", ROOT / "AGENTS.md"),
    ("atlas-memory-system-apps-checklist", "Atlas checklist", DOCS / "atlas-memory-system-apps-checklist.md"),
    ("unified-reliability-semantic-pipelines-checklist", "Reliability checklist", DOCS / "unified-reliability-semantic-pipelines-checklist.md"),
]

GALLERY = [
    ("workspace", "Workspace", "Architecture", "Home launches four applications that share SQLite and LanceDB."),
    ("tui-shell", "TUI shell", "Nested", "Chrome wraps the module body; overlays trap Tab and clicks."),
    ("recon-turn", "Recon turn", "Process", "Recall, directives, picker, binder, executor, synthesis."),
    ("recon-bindings", "Bindings", "Data flow", "Every input is grounded or the step is skipped."),
    ("tool-picker", "Tool picker", "Flowchart", "Jev decisions, chat JSON, or a deterministic fallback."),
    ("osint-providers", "OSINT catalog", "Nested", "Firecrawl, SociaVault, and Hunter are primary."),
    ("atlas-pipeline", "Atlas cycle", "Process", "Discovery, heat bands, then regional headlines."),
    ("intel-report", "Intel report", "Data flow", "Article to job to attempts to BLUF Summary."),
    ("intel-job-states", "Intel states", "State machine", "Queued, running, paused, then a terminal state."),
    ("brain-recall", "Brain recall", "Architecture", "SQLite plus optional MiniLM vectors."),
    ("model-roles", "Models and roles", "Architecture", "auth.json holds keys; config.toml holds roles."),
    ("persistence", "Persistence", "Layer stack", "TUI, core, SQLite, LanceDB, secrets."),
    ("schema-core", "Report tables", "Database schema", "intel_report_attempts is the dispatch ledger."),
    ("jobs-lifecycle", "Durable jobs", "State machine", "Register, lease, complete, or fail."),
    ("investigation-harness", "Harness", "Process", "Shared pipeline under Recon, Home, and Intel."),
    ("agent-graphify", "Agent exploration", "Process", "Query the graph, plan, edit, update."),
]


def slugify(text: str) -> str:
    s = text.strip().lower()
    s = re.sub(r"[^\w\s-]", "", s)
    s = re.sub(r"[\s_]+", "-", s)
    return s.strip("-") or "section"


def inline(text: str) -> str:
    parts: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        if text.startswith("`", i):
            end = text.find("`", i + 1)
            if end != -1:
                parts.append("<code>" + html.escape(text[i + 1 : end]) + "</code>")
                i = end + 1
                continue
        if text.startswith("[![", i):
            wrapped = re.match(
                r"\[!\[([^\]]*)\]\(([^)]+)\)\]\(([^)]+)\)", text[i:]
            )
            if wrapped:
                alt, src, href = (
                    wrapped.group(1),
                    rewrite_href(wrapped.group(2)),
                    rewrite_href(wrapped.group(3)),
                )
                parts.append(
                    f'<figure><a href="{html.escape(href)}"><img src="{html.escape(src)}" alt="{html.escape(alt)}"></a>'
                    f"<figcaption>{html.escape(alt)}</figcaption></figure>"
                )
                i += wrapped.end()
                continue
        if text.startswith("![", i):
            m = re.match(r"!\[([^\]]*)\]\(([^)]+)\)", text[i:])
            if m:
                alt, url = m.group(1), rewrite_href(m.group(2))
                if url.endswith(".html"):
                    parts.append(
                        f'<p class="figure"><a href="{html.escape(url)}">{html.escape(alt or url)}</a></p>'
                    )
                else:
                    parts.append(
                        f'<figure><a href="{html.escape(url)}"><img src="{html.escape(url)}" alt="{html.escape(alt)}"></a>'
                        f"<figcaption>{html.escape(alt)}</figcaption></figure>"
                    )
                i += m.end()
                continue
        if text.startswith("[", i):
            m = re.match(r"\[([^\]]+)\]\(([^)]+)\)", text[i:])
            if m:
                label, url = m.group(1), rewrite_href(m.group(2))
                parts.append(
                    f'<a href="{html.escape(url)}">{inline(label) if "`" in label else html.escape(label)}</a>'
                )
                i += m.end()
                continue
        if text.startswith("**", i):
            end = text.find("**", i + 2)
            if end != -1:
                parts.append("<strong>" + inline(text[i + 2 : end]) + "</strong>")
                i = end + 2
                continue
        if text.startswith("*", i) and not text.startswith("**", i):
            end = text.find("*", i + 1)
            if end != -1:
                parts.append("<em>" + inline(text[i + 1 : end]) + "</em>")
                i = end + 1
                continue
        ch = text[i]
        if ch == "<":
            parts.append("&lt;")
        elif ch == ">":
            parts.append("&gt;")
        elif ch == "&":
            parts.append("&amp;")
        else:
            parts.append(ch)
        i += 1
    return "".join(parts)


def rewrite_href(url: str) -> str:
    if url.startswith("http://") or url.startswith("https://") or url.startswith("mailto:"):
        return url
    path, hash_ = (url.split("#", 1) + [""])[:2]
    hash_ = f"#{hash_}" if hash_ else ""
    name = Path(path).name
    if path.endswith(".md"):
        stem = Path(name).stem
        if stem.lower() == "readme":
            if "docs" in path.replace("\\", "/") or path in ("README.md", "./README.md"):
                return f"index.html{hash_}" if "docs" in path.replace("\\", "/") else f"index.html{hash_}"
            return f"index.html{hash_}"
        if stem == "AGENTS":
            return f"agents.html{hash_}"
        return f"{stem}.html{hash_}"
    if "/diagrams/" in path.replace("\\", "/") or path.startswith("diagrams/"):
        return f"../diagrams/{name}{hash_}"
    if path.startswith("../"):
        # AGENTS.md, README.md already handled; leftover source files stay as text
        if name.endswith(".md"):
            return f"{Path(name).stem.lower()}.html{hash_}"
        return path + hash_
    return url


def convert(md: str) -> str:
    lines = md.replace("\r\n", "\n").split("\n")
    out: list[str] = []
    i = 0
    in_code = False
    code_lang = ""
    code_buf: list[str] = []
    used_ids: dict[str, int] = {}

    def heading_id(title: str) -> str:
        base = slugify(title)
        n = used_ids.get(base, 0)
        used_ids[base] = n + 1
        return base if n == 0 else f"{base}-{n + 1}"

    def flush_para(buf: list[str]) -> None:
        text = " ".join(s.strip() for s in buf).strip()
        if text:
            rendered = inline(text)
            if rendered.startswith("<figure"):
                out.append(rendered)
            else:
                out.append("<p>" + rendered + "</p>")
        buf.clear()

    para: list[str] = []
    while i < len(lines):
        line = lines[i]
        if in_code:
            if line.startswith("```"):
                lang = html.escape(code_lang)
                body = html.escape("\n".join(code_buf))
                out.append(f'<pre><code class="lang-{lang}">{body}</code></pre>')
                in_code = False
                code_buf = []
                code_lang = ""
            else:
                code_buf.append(line)
            i += 1
            continue
        if line.startswith("```"):
            flush_para(para)
            in_code = True
            code_lang = line[3:].strip()
            i += 1
            continue
        if re.match(r"^#{1,6} ", line):
            flush_para(para)
            level = len(line) - len(line.lstrip("#"))
            title = line[level + 1 :].strip()
            hid = heading_id(title)
            out.append(f'<h{level} id="{html.escape(hid)}">{inline(title)}</h{level}>')
            i += 1
            continue
        if re.match(r"^---+\s*$", line) or re.match(r"^\*\*\*+\s*$", line):
            flush_para(para)
            out.append("<hr>")
            i += 1
            continue
        if line.strip().startswith("|") and i + 1 < len(lines) and re.match(r"^\s*\|?\s*:?-{3,}", lines[i + 1]):
            flush_para(para)
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(lines[i])
                i += 1
            if len(rows) >= 2:
                def cells(row: str) -> list[str]:
                    row = row.strip()
                    if row.startswith("|"):
                        row = row[1:]
                    if row.endswith("|"):
                        row = row[:-1]
                    return [c.strip() for c in row.split("|")]

                head = cells(rows[0])
                body_rows = [cells(r) for r in rows[2:]]
                thead = "".join(f"<th>{inline(c)}</th>" for c in head)
                tbody = []
                for r in body_rows:
                    tds = "".join(f"<td>{inline(c)}</td>" for c in r)
                    tbody.append(f"<tr>{tds}</tr>")
                out.append("<div class='table-wrap'><table><thead><tr>" + thead + "</tr></thead><tbody>" + "".join(tbody) + "</tbody></table></div>")
            continue
        ul = re.match(r"^(\s*)[-*] (.+)$", line)
        ol = re.match(r"^(\s*)\d+\. (.+)$", line)
        if ul or ol:
            flush_para(para)
            ordered = bool(ol)
            items = []
            while i < len(lines):
                m = re.match(r"^(\s*)\d+\. (.+)$", lines[i]) if ordered else re.match(r"^(\s*)[-*] (.+)$", lines[i])
                if not m:
                    break
                items.append(m.group(2))
                i += 1
            tag = "ol" if ordered else "ul"
            out.append(f"<{tag}>" + "".join(f"<li>{inline(it)}</li>" for it in items) + f"</{tag}>")
            continue
        if not line.strip():
            flush_para(para)
            i += 1
            continue
        para.append(line)
        i += 1
    flush_para(para)
    if in_code:
        out.append("<pre><code>" + html.escape("\n".join(code_buf)) + "</code></pre>")
    return "\n".join(out)


NAV = [
    ("index.html", "Overview"),
    ("gallery.html", "Gallery"),
    ("usage.html", "Usage"),
    ("concepts.html", "Concepts"),
    ("architecture.html", "Architecture"),
    ("providers.html", "Providers"),
    ("conventions.html", "Conventions"),
    ("investigation-harness.html", "Harness"),
    ("jev-decision-gates.html", "Jev"),
    ("opencode-v2.html", "OpenCode"),
    ("ui-interaction-audit.html", "UI audit"),
    ("agents.html", "Agents"),
]


def chrome(title: str, current: str, body: str, extra_class: str = "") -> str:
    links = []
    for href, label in NAV:
        cls = ' class="on"' if href == current else ""
        links.append(f'<a href="{href}"{cls}>{html.escape(label)}</a>')
    nav = "\n        ".join(links)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{html.escape(title)} · Argos docs</title>
  <link rel="stylesheet" href="site.css">
  <link href="https://fonts.googleapis.com/css2?family=Instrument+Serif:ital@0;1&family=Geist:wght@400;500;600&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
</head>
<body>
  <header class="top">
    <a class="brand" href="index.html">Argos OSINT</a>
    <nav>
        {nav}
    </nav>
  </header>
  <main class="doc {extra_class}">
{body}
  </main>
  <footer>
    <p>Open <code>docs/index.html</code> in a browser. Figures: <a href="gallery.html">gallery</a> · source Markdown stays in <code>docs/</code>.</p>
  </footer>
</body>
</html>
"""


CSS = """
:root {
  --paper: #f5f5f5;
  --paper-2: #ececec;
  --ink: #2d3142;
  --muted: #4f5d75;
  --soft: #7a8399;
  --accent: #eb6c36;
  --link: #2e5aa8;
  --rule: rgba(45,49,66,0.12);
}
* { box-sizing: border-box; }
html, body { margin: 0; background: var(--paper); color: var(--ink); font-family: Geist, system-ui, sans-serif; line-height: 1.55; }
.top {
  position: sticky; top: 0; z-index: 4;
  display: flex; flex-wrap: wrap; align-items: center; gap: 12px 20px;
  padding: 12px 24px; background: var(--paper); border-bottom: 1px solid var(--rule);
}
.brand { font-family: "Instrument Serif", serif; font-size: 1.35rem; color: var(--ink); text-decoration: none; }
.top nav { display: flex; flex-wrap: wrap; gap: 4px 14px; }
.top nav a { color: var(--muted); text-decoration: none; font-size: 13px; font-weight: 500; }
.top nav a:hover, .top nav a.on { color: var(--accent); }
main.doc { max-width: 920px; margin: 0 auto; padding: 32px 24px 80px; }
main.doc.wide { max-width: 1100px; }
h1, h2, h3, h4 { font-family: "Instrument Serif", serif; font-weight: 400; line-height: 1.25; }
h1 { font-size: 2rem; margin: 0 0 1rem; }
h2 { font-size: 1.45rem; margin: 2rem 0 0.75rem; padding-top: 0.5rem; border-top: 1px solid var(--rule); }
h3 { font-size: 1.2rem; margin: 1.5rem 0 0.5rem; }
p { margin: 0 0 1rem; }
a { color: var(--link); }
code, pre { font-family: "Geist Mono", ui-monospace, monospace; font-size: 0.86em; }
code { background: var(--paper-2); padding: 0.1em 0.35em; border-radius: 4px; }
pre { background: var(--paper-2); padding: 16px; overflow-x: auto; border-radius: 6px; border: 1px solid var(--rule); }
pre code { background: none; padding: 0; }
.table-wrap { overflow-x: auto; margin: 0 0 1.25rem; }
table { border-collapse: collapse; width: 100%; font-size: 14px; }
th, td { text-align: left; vertical-align: top; padding: 8px 10px; border-bottom: 1px solid var(--rule); }
th { font-size: 12px; letter-spacing: 0.04em; text-transform: uppercase; color: var(--muted); font-weight: 600; }
figure { margin: 1.25rem 0 1.75rem; }
figure img { max-width: 100%; height: auto; background: var(--paper); border: 1px solid var(--rule); border-radius: 6px; }
figcaption { font-family: "Geist Mono", ui-monospace, monospace; font-size: 11px; color: var(--muted); margin-top: 8px; }
.lede { color: var(--muted); font-size: 1.05rem; max-width: 40rem; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 16px; margin: 1.5rem 0 2rem; }
.card { display: block; background: #fff; border: 1px solid var(--rule); border-radius: 6px; padding: 16px; text-decoration: none; color: inherit; }
.card:hover { border-color: var(--accent); }
.card .kicker { font-family: "Geist Mono", ui-monospace, monospace; font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--accent); }
.card h2 { font-size: 1.15rem; margin: 0.35rem 0 0.4rem; border: 0; padding: 0; }
.card p { color: var(--muted); font-size: 14px; margin: 0; }
.gallery { display: grid; gap: 40px; }
.gallery article h2 { border: 0; margin: 0 0 0.35rem; padding: 0; }
.gallery article .kicker { font-family: "Geist Mono", ui-monospace, monospace; font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--muted); }
.gallery article img { width: 100%; height: auto; background: var(--paper); border: 1px solid var(--rule); border-radius: 6px; }
footer { max-width: 920px; margin: 0 auto; padding: 0 24px 48px; color: var(--soft); font-size: 13px; }
ul, ol { margin: 0 0 1rem; padding-left: 1.25rem; }
hr { border: 0; border-top: 1px solid var(--rule); margin: 2rem 0; }
"""


def overview_body() -> str:
    cards = []
    for href, label in NAV[1:8]:
        cards.append(
            f'<a class="card" href="{href}"><div class="kicker">Doc</div><h2>{html.escape(label)}</h2><p>Open the HTML page.</p></a>'
        )
    thumbs = []
    for slug, title, kind, lede in GALLERY[:8]:
        thumbs.append(
            f'''<a class="card" href="gallery.html#{slug}">
              <div class="kicker">{html.escape(kind)}</div>
              <h2>{html.escape(title)}</h2>
              <p>{html.escape(lede)}</p>
            </a>'''
        )
    return f"""
    <p class="kicker" style="font-family:'Geist Mono',ui-monospace,monospace;font-size:11px;letter-spacing:.18em;text-transform:uppercase;color:var(--muted)">Documentation</p>
    <h1>Argos OSINT</h1>
    <p class="lede">A terminal investigation workspace. This site is the browser copy of the repo docs: every Markdown page, plus every diagram. Open this file locally; no server is required.</p>
    <p>Product tour in the repo <a href="index.html">overview</a>. Source of truth for investigation mechanics: <a href="architecture.html">architecture</a>. Keys: <a href="providers.html">providers</a>.</p>
    <h2>Read</h2>
    <div class="cards">{''.join(cards)}</div>
    <h2>Figures</h2>
    <p>Sixteen editorial diagrams. The <a href="gallery.html">gallery</a> shows all of them with captions. Click a card or a figure to open the standalone page.</p>
    <div class="cards">{''.join(thumbs)}</div>
    <p><a href="gallery.html">All figures →</a></p>
    <h2>Also in this site</h2>
    <ul>
      <li><a href="agents.html">AGENTS.md</a> — graphify, planning-with-files, checks</li>
      <li><a href="jev-decision-gates.html">Jev decision gates</a></li>
      <li><a href="opencode-v2.html">OpenCode V2</a></li>
      <li><a href="ui-interaction-audit.html">UI interaction audit</a></li>
      <li><a href="atlas-memory-system-apps-checklist.html">Atlas / Brain checklist</a> (historical)</li>
      <li><a href="unified-reliability-semantic-pipelines-checklist.html">Reliability checklist</a> (historical)</li>
    </ul>
"""


def gallery_body() -> str:
    arts = []
    for slug, title, kind, lede in GALLERY:
        arts.append(
            f'''<article id="{html.escape(slug)}">
              <div class="kicker">{html.escape(kind)}</div>
              <h2>{html.escape(title)}</h2>
              <p>{html.escape(lede)}</p>
              <a href="../diagrams/{slug}.html"><img src="../diagrams/{slug}.svg" alt="{html.escape(title)}"></a>
            </article>'''
        )
    return "<h1>Gallery</h1><p class='lede'>Every Argos figure, in document order. Click a drawing for the full HTML page (title, type, and SVG).</p><div class='gallery'>" + "".join(arts) + "</div>"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "site.css").write_text(CSS, encoding="utf-8")

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    # Drop the raw SVG embeds from README; the gallery covers them.
    readme = re.sub(r"\[!\[.*?\]\(docs/diagrams/.*?\)\]\(docs/diagrams/.*?\)\n*", "", readme)
    overview = convert(readme)
    (OUT / "index.html").write_text(
        chrome("Overview", "index.html", overview_body() + "<hr>" + overview, "home"),
        encoding="utf-8",
    )
    (OUT / "gallery.html").write_text(
        chrome("Gallery", "gallery.html", gallery_body(), "wide"),
        encoding="utf-8",
    )

    for slug, title, path in PAGES:
        if slug in ("index", "gallery"):
            continue
        assert path is not None
        md = path.read_text(encoding="utf-8")
        body = convert(md)
        (OUT / f"{slug}.html").write_text(
            chrome(title, f"{slug}.html", body),
            encoding="utf-8",
        )

    # Convenience copy at docs/index.html
    (DOCS / "index.html").write_text(
        """<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="0; url=html/index.html">
  <title>Argos docs</title>
  <link rel="canonical" href="html/index.html">
</head>
<body>
  <p><a href="html/index.html">Open the Argos documentation</a>.</p>
</body>
</html>
""",
        encoding="utf-8",
    )
    print("wrote", OUT)


if __name__ == "__main__":
    main()
