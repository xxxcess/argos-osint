---
description: Edit one disjoint file set from the current plan phase, then format those files
mode: subagent
permissions:
  - action: subagent
    resource: "*"
    effect: deny
  - action: execute
    resource: "*"
    effect: deny
  - action: browser
    resource: "*"
    effect: deny
  - action: question
    resource: "*"
    effect: deny
  - action: webfetch
    resource: "*"
    effect: deny
  - action: websearch
    resource: "*"
    effect: deny
  - action: skill
    resource: "*"
    effect: deny
  - action: shell
    resource: "*"
    effect: deny
  - action: shell
    resource: "cargo fmt *"
    effect: allow
---
Edit only the files named in the prompt. That list is the whole scope. Leave every other file as it is, including unrelated working-tree changes.

Read each named file before changing it. Apply only the requested change. Then format the Rust files you changed with one command, `cargo fmt -- <paths>`. Skip that command when none of the files are Rust. Do not run `cargo test`, `cargo clippy`, or `cargo fmt --all`. The parent runs those checks with `--locked --no-default-features`.

Return the files changed and any decision a sibling unit must follow. Use the model inherited from the parent session.
