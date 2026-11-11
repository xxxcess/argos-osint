//! Input and bounded text for the line-oriented clients.

use std::io::BufRead;
use std::io::Write;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::Operation;

#[derive(Clone, Copy)]
pub(crate) struct Session<'a> {
    pub client: &'a Client,
    pub runtime: &'a Runtime,
}

impl Session<'_> {
    pub fn request(&self, operation: impl Into<Operation>) -> Result<Value> {
        self.runtime.block_on(operation.into().request(self.client))
    }
}

pub(crate) fn run<I: BufRead, O: Write>(
    mut input: I,
    mut output: O,
    label: &str,
    mut execute: impl FnMut(&[&str], &mut I, &mut O) -> Result<()>,
) -> Result<()> {
    while let Some(line) = prompt(&mut input, &mut output, label)? {
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            [] => (),
            ["quit"] => break,
            _ => {
                if let Err(error) = execute(&words, &mut input, &mut output) {
                    writeln!(output, "{error}")?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> Result<Option<String>> {
    write!(output, "{label}")?;
    output.flush()?;
    let mut line = String::new();
    Ok((input.read_line(&mut line)? != 0).then_some(line))
}

pub(crate) fn confirmed(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
) -> Result<bool> {
    let answer = prompt(input, output, question)?;
    Ok(answer.as_deref().map(str::trim) == Some("yes"))
}

/// Split captured writer output into plain lines for the full-screen detail pane.
pub(crate) fn lines(buffer: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(buffer)
        .lines()
        .map(str::to_owned)
        .collect()
}

pub(crate) fn rows<'a>(reply: &'a Value, key: &str) -> Result<&'a [Value]> {
    reply[key]
        .as_array()
        .map(Vec::as_slice)
        .context("invalid API row response")
}

pub(crate) fn field<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or_default()
}

pub(crate) fn write_field(
    output: &mut impl Write,
    label: &str,
    row: &Value,
    key: &str,
    max_chars: usize,
) -> Result<()> {
    let value = field(row, key);
    if !value.is_empty() {
        writeln!(output, "  {label}  {}", text(value, max_chars))?;
    }
    Ok(())
}

pub(crate) fn text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut text: String = chars
        .by_ref()
        .take(max_chars)
        .flat_map(char::escape_debug)
        .collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

pub(crate) fn strings(reply: &Value, key: &str, max_items: usize) -> String {
    reply[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(max_items)
        .map(|value| text(value, 128))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn row_index(selector: &str) -> Option<usize> {
    selector
        .strip_prefix('#')?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}
