---
name: argos-rust-style
description: Apply Google's Rust Style Guide when writing, reviewing, or refactoring any Rust in this repo. Use for API design, naming, imports, error handling, documentation, and implementation patterns.
---

# Rust style — Google Rust Style Guide

Load this for every Rust edit or review under `crates/`. Follow the public [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/) and [API Guidelines](https://rust-lang.github.io/api-guidelines/) unless this skill conflicts. `cargo fmt` handles formatting.

## Argos overrides

These win when the guide below disagrees:

- Tokio async is required. [Async runtime and process safety](../../../docs/wiki/code/conventions.md#async-runtime-and-process-safety) overrides the Chromium ban on `async`/`await`.
- Keep the existing workspace. Do not flatten crates, ban module subdirectories, or adopt Chromium `rust_static_library` layout.
- `argos-osint-core` and `argos-osint-api` are library code. `argos-osint-bin` and `argos-api-host` are application code.
- Fuchsia sync primitives do not apply.
- Tests stay offline. Do not add unstable features.

## Core principles

- Prioritize readability and maintainability over writeability.
- Favor automation and consistency for large-scale changes.
- Align with the wider Rust ecosystem.
- Rules must pull their weight.
- Library code follows the binding API rules. Application code treats them as recommendations and favors simplicity.

## Code organization

- Name source files and leaf modules after the primary item (`mod foo;` → `foo.rs`).
- Non-leaf modules hold `mod` declarations and re-exports, plus small core code, `mod test`, or `#[cxx::bridge]` when those already exist.
- No glob imports except `use super::*` inline, `use enum::*` for enum methods, and `pub use submodule::*` from a non-leaf module.
- Each `use` imports from one module. No `::` inside `{}`.
- Prefer locally scoped trait imports, inside the function unless the trait is used throughout the module.
- Qualify top-level function calls with the module name (`mem::transmute`). Exceptions: disambiguated names, small first-party modules, and collision-prone modules.
- Import type names and use them unqualified (`use regex::Regex;` then `Regex`). Exceptions: ambiguities, large groups, macros, and generated code.

## Function inputs and outputs

- Accept flexible references for immutable data: `&T` not `&Box<T>`, `&[T]` not `&Vec<T>`, `&str` not `&String`, `Option<&T>` not `&Option<T>`.
- Library: prefer `impl Iterator` returns and `impl IntoIterator` params. Application: prefer `Vec<T>` / `&[T]` when ownership or random access is needed.
- Consider `impl Into<T>` for library flexibility. Avoid it in application code.
- Prefer `enum` over `bool` when the purpose is not obvious. Document bools.
- `Option` for an obvious missing value. `Result` for a recoverable error that needs context. `panic!` for a violated invariant; document it.

## Types

- Prefer newtypes over aliases when the difference is semantic, needs its own API, or should stay distinct.
- Prefer named fields over tuples and non-newtype tuple structs. Exceptions: newtypes and very local values.

## Traits

- Add impls proactively in libraries and as needed in applications.
- Implement standard traits instead of custom methods (`FromStr` rather than only a custom `from_str`). A custom method may remain for discoverability.
- Prefer `#[derive]` over manual impls.
- Library functions prefer generics (`T: Trait` / `impl Trait`) over `dyn Trait`, except where code size matters.
- Define a trait for generics, `impl`/`dyn Trait`, or an extension. Otherwise use an inherent impl.
- Extension traits use an `Ext` suffix and are owned by the team that owns the original type.
- Public fields: implement the traits of the subfields. Errors: `Copy` when possible. `Clone` unless the type owns an external resource. `Default` when there is a `new()` or a real default. Prefer `PartialEq`/`PartialOrd` unless `Eq`/`Ord` requirements hold. Map keys need `Hash` + `Eq`, and `Ord` when a total order exists.
- Pair important `From` impls with an explicit constructor (`new`, `from_foo`, `try_new`).
- Options structs (all-public fields) implement `Default`.

## Concurrency

Never observe or continue through a poisoned mutex. Argos uses Tokio; do not replace it with `std` threads or remove `async`.

## Macros

Prefer, in order: ordinary Rust, `macro_rules!`, other codegen, then `proc_macro`. Weigh tooling and large-scale-change cost.

## Naming

- Methods follow std style. Use `as_` / `to_` / `into_` with the matching `self`. Getters: `foo()` (`&T`, or `T` if `Copy`), `foo_mut()`, `take_foo()`, `set_foo()`, `with_foo()` / `with_mut_foo()`, `replace_foo()`.
- Lifetimes are descriptive. Exceptions: highly generic code, `'_`, and up to two single-character local relations.
- Generics are short and indicative (`T` / `U` when unbound; a letter from the main trait otherwise).
- Initialization: `new()` / `new_with_foo()` / `try_new()`; `from_foo()` alongside `From`.
- Avoid abbreviations unknown outside the project. Common public abbreviations such as `IpAddr` may stay.

## Documentation

- Document public APIs. Include examples for crate usage.
- Document panic cases for malformed input and state unsafe invariants.
- Use capitalization and punctuation.
- Comments add insight the code does not already provide.

## Other features

- Unsafe needs a reason a safe equivalent will not do. Benchmark it when the reason is performance.
- Implement expected API sets together (`len` with `is_empty`).
- Do not call `#[doc(hidden)]` items you do not own.
- `const` only when the item is expected to stay const.
- Operator overloads only for mathematical or logical meaning.
- Public enums and public-field structs used across crate boundaries are `#[non_exhaustive]` unless a change is impossible or a large-scale change is already committed. Skip `non_exhaustive` when functional record update is required and the struct is not a cross-team contract.
- `#[must_use]` on pure functions and on error or critical returns whose type is not already marked. Also on error types and lazy computations.
- No code before `main()` (`ctor` and similar).

## Implementation

- Leave enough explicit types for a reader who knows the codebase. Prefer the annotation on the variable over turbofish.
- Prefer pattern matching over check-then-unwrap. Prefer `expect` over `unwrap` when stating an invariant.
- On `#[non_exhaustive]` enums, list variants or use `if let` / `let else`.
- `assert!` messages say why, or log the values, especially for API misuse.
- Prefer a direct `for` loop when an iterator chain is hard to read. Do not put a chain in the `for` header, and avoid `for_each` unless it is clearer.
- Do not call `panic::catch_unwind`.
- Prefer `format!` / `write!` for concatenation and substitution.

## Review checklist

| Area | Check |
| --- | --- |
| Imports | No globs; one module per `use`; qualify functions; unqualified types |
| Errors | `Option` / `Result` / panic used for the right case; panics documented |
| Types | Newtypes where the meaning differs; named fields |
| Traits | Derive when possible; extension suffix `Ext`; `Default` on options structs |
| Naming | Descriptive lifetimes; no obscure abbreviations |
| Public APIs | Docs, `non_exhaustive` where needed, `must_use` |
| Impl | Pattern match over unwrap; context on asserts |
| Argos | Async and process rules kept; crate layout unchanged |
