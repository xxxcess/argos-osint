---
name: argos-rust-style
description: Apply Google's Rust Style Guide when writing, reviewing, or refactoring any Rust in this repo. Use for API design, naming, imports, error handling, documentation, and implementation patterns.
---

# Rust style — Google Rust Style Guide

Follow the [agent workflow](../../../docs/wiki/code/agent-workflow.md), [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/), and [API Guidelines](https://rust-lang.github.io/api-guidelines/) for every Rust edit/review under `crates/`. `cargo fmt` handles formatting.

## Argos overrides

- **Async & stack:** Tokio async is required ([async/process safety](../../../docs/wiki/code/conventions.md#async-runtime-and-process-safety) overrides Chromium's `async`/`await` ban). Follow its allocation-boundary rule for large task-local callbacks and verify stack-overflow fixes on the normal public stack—never by enlarging test stacks.
- **Workspace & crates:** Keep the existing workspace layout (do not flatten crates, ban module subdirectories, or adopt `rust_static_library`). Library crates (`argos-osint-core`, `argos-osint-api`) follow binding API rules; application crates (`fleetworks-desktop`, `fleetworks-search-index`, `argos-api-host`) treat them as recommendations favoring simplicity.
- **Environment:** Fuchsia sync primitives do not apply; tests stay offline; no unstable features. Never run Rust mutation tests (`cargo mutants`) locally—use hosted `mutation.yml` and report local mutation as skipped.

## Core & organization

- Prioritize readability, maintainability, ecosystem alignment, and rules that pull their weight.
- Name leaf files after their primary item (`mod foo;` → `foo.rs`); non-leaf modules hold `mod`/re-exports plus small core code, `mod test`, or existing `#[cxx::bridge]`.
- **Imports:** No globs except `use super::*` inline, `use enum::*` in enum methods, and `pub use sub::*` from non-leaf modules. One module per `use` (no `::` inside `{}`). Scope trait imports locally when used only in one function. Qualify free functions (`mem::transmute`) unless unambiguous/small first-party; import types unqualified (`Regex`) unless ambiguous/macro/generated.

## APIs, types, traits, and naming

- **Signatures:** Accept `&T`, `&[T]`, `&str`, `Option<&T>` over `&Box<T>`/`&Vec<T>`/`&String`/`&Option<T>`. Libraries prefer `impl Iterator`/`impl IntoIterator` and `impl Into<T>`; apps prefer `Vec<T>`/`&[T]` and concrete types. Prefer `enum` over unclear `bool` (document bools). Use `Option` for obvious absence, `Result` for recoverable errors with context, and documented `panic!` only for violated invariants.
- **Types:** Prefer newtypes for distinct semantics and named fields over tuples (except newtypes and tiny locals).
- **Traits:** Add impls proactively in libraries, as needed in apps. Prefer `#[derive]`, standard traits (`FromStr`) over custom-only methods, generics over `dyn Trait` in libraries (unless code size matters), and inherent impls unless polymorphism/extension (`Ext` suffix, owned with the type) is needed. Public-field types mirror subfield traits; errors `Copy` when possible; `Clone` unless owning external resources; `Default` on options structs or real defaults; `PartialEq`/`PartialOrd` unless `Eq`/`Ord` hold; map keys need `Hash + Eq` (and `Ord` if total). Pair key `From` impls with explicit constructors (`new`, `from_foo`, `try_new`).
- **Naming:** Follow std conventions (`as_`/`to_`/`into_`, `foo()`, `foo_mut()`, `take_foo()`, `set_foo()`, `with_foo()`/`with_mut_foo()`, `replace_foo()`, `new`/`new_with_foo`/`try_new`/`from_foo`). Use descriptive lifetimes (except `'_`, highly generic code, or ≤2 local relations), short indicative generics (`T`/`U` or trait initial), and no obscure abbreviations (common `IpAddr` is fine).
- **Docs & attributes:** Document public APIs (with examples for crates), panic conditions, and `unsafe` invariants with capitalized, punctuated prose; comments explain *why*. Require a compelling reason (plus benchmarks for perf) before `unsafe`. Pair related APIs (`len`/`is_empty`). Never call external `#[doc(hidden)]` items or pre-`main` `ctor`s. Use `const` only if meant to stay const; operator overloads only for math/logic; `#[non_exhaustive]` on cross-crate public enums/structs (skip when functional update is needed internally); `#[must_use]` on pure functions, error types, lazy values, and critical returns.

## Concurrency, macros, and implementation

- Never continue through a poisoned mutex; keep Tokio async (no `std` thread replacements).
- Macro preference order: ordinary Rust → `macro_rules!` → other codegen → `proc_macro`.
- Annotate variables rather than turbofish; pattern-match over check-then-unwrap; prefer `expect` with invariant over `unwrap`; match `#[non_exhaustive]` with listed variants or `if let`/`let else`. Add context to `assert!`. Prefer direct `for` loops over unreadable iterator chains (no chains in `for` headers; avoid `for_each` unless clearer). Never call `panic::catch_unwind`. Prefer `format!`/`write!` for string assembly.
