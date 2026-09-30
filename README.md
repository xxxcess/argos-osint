# Argos OSINT

Argos is a terminal shell for memory recall and provider connections. It keeps the dark teal theme, left app launcher, main canvas, and bottom composer. The apps are **Brain**, **Providers**, and **System**.

## Run

```sh
cargo run -p argos-osint-bin
```

Click an app, tab, button, list item, or input field to use it. Tab and Shift+Tab move focus, Enter activates the focused control, F1–F3 opens an app, Esc returns to the launcher, and Ctrl+C quits. Text fields support cursor movement and editing.

## Brain

Brain stores insights that future chat apps can contribute and retrieve. Every memory has a source app and conversation ID. A message ID and deep link are optional. The core API is `Store::add_memory(text, category, pinned, MemorySource)` and `Store::recall(query, limit)`. Recall results include provenance.

```sh
argos remember --app future-chat --conversation thread-123 --message msg-456 --category project "Building the Atlas launch"
argos recall "What am I building?"
argos memories
```

In Brain, fill Source app, Conversation ID, and Insight, then click **Save insight**. Enter a Recall question and click **Recall**. Click a memory to select it, then use **Pin / unpin** or **Delete**. The composer also accepts `add <app> <conversation-id> | [category:] insight`, `recall <question>`, `pin`, and `delete`.

Categories are fact, identity, preference, contact, project, goal, and task.

## Providers

Providers has **Grok**, **OpenAI**, **OpenRouter**, and **Models** tabs. Grok and OpenAI offer **Sign in** and **Check existing login** buttons that run their subscription flows in the background and display progress in the app. Grok uses Grok Build CLI; OpenAI uses ChatGPT sign-in through Codex CLI. OpenRouter has a masked API key field, **Verify connection**, **Save key**, and an optional HTTPS endpoint field. Verification checks the current draft without saving it. Models assigns the Writer provider and model.

`argos login`, `argos models`, and `argos logout` remain available from the command line. [Provider details](docs/providers.md).

## State and migration

State lives in `~/.argos` (`ARGOS_HOME` overrides it):

| Path | Contents |
| --- | --- |
| `argos.db` | Brain memories and source metadata |
| `config.toml` | Writer provider, writer model, and modality |
| `auth.json` | Provider credentials, owner only on Unix |
| `hardware.json` | Cached host profile |

Opening the store migrates an older database to the Brain schema. Ordinary old memories receive `argos-legacy / unknown` provenance because the old schema did not record their conversation. Report linked memories and the old case, report, and research tables are removed. Old configuration and Gmail credential fields are removed when their files are loaded.

## Verify

```sh
cargo test --workspace
```
