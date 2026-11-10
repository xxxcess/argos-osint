//! Interactive Brain reads over retained actor, brief, and topology contracts.

use std::io::BufRead;
use std::io::Write;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::terminal;
use crate::terminal::write_field;
use crate::terminal::Session;
use crate::BrainCommand;

const HELP: &str = "find NAME            Look up an exact retained actor name
aliases PREFIX       Look up exact and prefix aliases
actor ID | #N        Show an actor dossier by ID or from the last result
brief RUN ID         Show a retained article brief
topology SEED        Show recorded nodes and edges for a memory ID or brief:RUN/ARTICLE
help | quit          Show commands or close this session";
const MAX_VISIBLE: usize = 30;

pub(crate) fn run(
    client: &Client,
    runtime: &Runtime,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let session = Session { client, runtime };
    let mut actor_results = Vec::new();
    writeln!(output, "Fleetworks Brain\n{HELP}")?;
    terminal::run(input, output, "brain> ", |words, _, output| {
        match words {
            ["help"] => writeln!(output, "{HELP}")?,
            [command @ ("find" | "aliases"), terms @ ..] if !terms.is_empty() => {
                actor_results.clear();
                let reply = session.request(BrainCommand::Search {
                    query: terms.join(" "),
                    alias: *command == "aliases",
                    limit: 10,
                })?;
                actor_results = write_search(&reply, output)?;
            }
            ["actor", id] => {
                let id = if id.starts_with('#') {
                    terminal::row_index(id)
                        .and_then(|index| actor_results.get(index))
                        .map(String::as_str)
                        .context("no such row in the last actor search")?
                } else {
                    *id
                };
                let reply = session.request(BrainCommand::Actor { id: id.into() })?;
                write_actor(&reply, output)?;
            }
            ["brief", run, id] => {
                let reply = session.request(BrainCommand::Brief {
                    run: (*run).into(),
                    id: (*id).into(),
                })?;
                write_brief(&reply, output)?;
            }
            ["topology", seed] => {
                let reply = session.request(BrainCommand::Topology {
                    seed: (*seed).into(),
                })?;
                write_topology(&reply, output)?;
            }
            _ => bail!("unknown command or wrong number of arguments. Type help for commands."),
        }
        Ok(())
    })
}

fn write_search(reply: &Value, output: &mut impl Write) -> Result<Vec<String>> {
    let rows = terminal::rows(reply, "rows")?;
    let diagnostic = terminal::field(reply, "diagnostic");
    if !diagnostic.is_empty() {
        writeln!(
            output,
            "Actor discovery  {}",
            terminal::text(diagnostic, 128)
        )?;
    } else if rows.is_empty() {
        writeln!(output, "No matching retained actors.")?;
    }
    write_field(output, "Catalog revision", reply, "catalog_revision", 160)?;
    let mut ids = Vec::new();
    for row in rows {
        let id = terminal::field(row, "actor_id");
        if id.is_empty() {
            bail!("invalid actor search result");
        }
        ids.push(id.to_owned());
        let label = terminal::text(terminal::field(row, "label"), 240);
        let kind = terminal::text(terminal::field(row, "kind"), 80);
        let reason = terminal::text(terminal::field(row, "reason"), 80);
        let ambiguity = if row["ambiguous"] == true {
            " · ambiguous"
        } else {
            ""
        };
        writeln!(
            output,
            "#{}  {label}  [{kind} · {reason}{ambiguity}]  ({})",
            ids.len(),
            terminal::text(id, 256),
        )?;
        write_field(output, "Catalog ID", row, "catalog_id", 256)?;
        let provenance = &row["alias_provenance"];
        write_field(output, "Alias catalog ID", provenance, "catalog_id", 256)?;
        write_field(output, "Alias source ID", provenance, "source_id", 256)?;
    }
    if reply["truncated"] == true {
        writeln!(
            output,
            "Use the one-shot search limit to request more candidates."
        )?;
    }
    Ok(ids)
}

pub(crate) fn write_brief(brief: &Value, output: &mut impl Write) -> Result<()> {
    let article = &brief["article"];
    if !article.is_object() {
        bail!("invalid API brief response");
    }
    let title = terminal::text(terminal::field(article, "title"), 240);
    let run = terminal::text(terminal::field(article, "run_id"), 512);
    let id = terminal::text(terminal::field(article, "id"), 512);
    writeln!(output, "{title}\n  Article {run}/{id}")?;
    write_field(output, "Source", article, "source_name", 128)?;
    write_field(output, "Published", article, "published_at", 64)?;
    write_field(output, "URL", article, "url", 1024)?;
    write_field(output, "Description", article, "description", 1000)?;
    write_field(output, "Lifecycle", brief, "lifecycle", 128)?;
    write_field(output, "Status", brief, "status", 240)?;
    write_field(output, "Job", brief, "job_id", 512)?;
    write_field(output, "Confidence", brief, "confidence_grade", 64)?;
    for (key, label, text_field) in [
        ("actors", "Actors", "display_name"),
        ("claims", "Claims", "text"),
        ("inferences", "Inferences", "text"),
    ] {
        let items = brief[key].as_array().map(Vec::as_slice).unwrap_or_default();
        writeln!(output, "{label} ({})", items.len())?;
        for item in items.iter().take(MAX_VISIBLE) {
            let name = terminal::text(terminal::field(item, text_field), 360);
            let id = terminal::text(terminal::field(item, "id"), 512);
            writeln!(output, "  {name}  ({id})")?;
        }
        write_more(items.len(), output)?;
    }
    Ok(())
}

fn write_actor(actor: &Value, output: &mut impl Write) -> Result<()> {
    let name = terminal::text(terminal::field(actor, "display_name"), 200);
    let id = terminal::text(terminal::field(actor, "id"), 512);
    if id.is_empty() {
        bail!("invalid API actor dossier");
    }
    writeln!(output, "{name}\n  Actor {id}")?;
    write_field(output, "Subtype", actor, "subtype", 80)?;
    write_field(output, "Identity", actor, "identity_state", 80)?;
    write_field(output, "Description", actor, "description", 1000)?;
    let aliases = terminal::strings(actor, "aliases", 20);
    if !aliases.is_empty() {
        writeln!(output, "  Aliases  {aliases}")?;
    }
    for item in terminal::rows(actor, "metadata")?.iter().take(MAX_VISIBLE) {
        let key = terminal::text(terminal::field(item, "key"), 80);
        write_field(output, &key, item, "value", 240)?;
    }
    for (key, label) in [
        ("claims", "Claims"),
        ("briefs", "Briefs"),
        ("inferences", "Inferences"),
    ] {
        let items = actor[key].as_array().map(Vec::as_slice).unwrap_or_default();
        writeln!(output, "{label} ({})", items.len())?;
        for item in items.iter().take(MAX_VISIBLE) {
            let node = &item["node"];
            let name = terminal::text(terminal::field(node, "label"), 240);
            let id = terminal::text(terminal::field(node, "id"), 512);
            let relationship = terminal::text(terminal::field(&item["relationship"], "label"), 80);
            writeln!(output, "  {name}  [{relationship}]  ({id})")?;
            if let (Some(run), Some(id)) = (
                item["article"]["run_id"].as_str(),
                item["article"]["id"].as_str(),
            ) {
                writeln!(
                    output,
                    "    brief {} {}",
                    terminal::text(run, 512),
                    terminal::text(id, 512)
                )?;
            }
        }
        write_more(items.len(), output)?;
    }
    if actor["truncated"] == true {
        writeln!(output, "The server bounded this dossier.")?;
    }
    write_gaps(actor, output)
}

fn write_topology(graph: &Value, output: &mut impl Write) -> Result<()> {
    let nodes = terminal::rows(graph, "nodes")?;
    let edges = terminal::rows(graph, "edges")?;
    let label = terminal::text(terminal::field(graph, "root_label"), 240);
    writeln!(output, "{label}\nNodes ({})", nodes.len())?;
    for node in nodes.iter().take(MAX_VISIBLE) {
        let kind = terminal::text(terminal::field(node, "kind"), 80);
        let label = terminal::text(terminal::field(node, "label"), 240);
        let id = terminal::text(terminal::field(node, "id"), 512);
        writeln!(output, "  [{kind}] {label}  ({id})")?;
    }
    write_more(nodes.len(), output)?;
    writeln!(output, "Edges ({})", edges.len())?;
    for edge in edges.iter().take(MAX_VISIBLE) {
        let from = terminal::text(terminal::field(edge, "from"), 512);
        let to = terminal::text(terminal::field(edge, "to"), 512);
        let label = terminal::text(terminal::field(edge, "label"), 128);
        writeln!(output, "  {from} —{label}→ {to}")?;
    }
    write_more(edges.len(), output)?;
    if graph["has_continuation"] == true {
        writeln!(output, "The server bounded this graph.")?;
    }
    write_gaps(&graph["findings"], output)
}

fn write_gaps(record: &Value, output: &mut impl Write) -> Result<()> {
    let gaps = terminal::strings(record, "gaps", MAX_VISIBLE);
    if !gaps.is_empty() {
        writeln!(output, "Coverage gaps  {gaps}")?;
    }
    Ok(())
}

fn write_more(count: usize, output: &mut impl Write) -> Result<()> {
    if count > MAX_VISIBLE {
        writeln!(
            output,
            "  {} more in the one-shot JSON response.",
            count - MAX_VISIBLE
        )?;
    }
    Ok(())
}
