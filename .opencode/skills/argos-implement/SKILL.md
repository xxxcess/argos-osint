---
name: argos-implement
description: On the build agent, edit the current plan phase through parallel ecc-edit subagents, then test in the parent.
---

Run this on the `build` agent. `plan` does not launch `ecc-edit`.

1. Read the in-progress phase in `.planning/<id>/task_plan.md`. Split its files into disjoint sets. One set is one unit. A file that several units need belongs to one unit only.
2. Call `subagent` once per unit. Two or more units go in the same turn with `background` true. One unit stays in the foreground.

```json
{
  "agent": "ecc-edit",
  "description": "Edit binder inputs",
  "prompt": "Phase, the exact file list, the change, and constraints. The child has no other context.",
  "background": true
}
```

Do not set `model`. Do not poll. Do not edit those files in the parent while a child owns them.

3. After the children finish, run the checks in this session with shell `timeout` set to `600000` and `ARGOS_EMBED` unset:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

4. On failure, map each error to the unit that owns the file. Launch `ecc-edit` again the same way. Pass `sessionID` to continue the editor that already owns those files, and put the failing command plus the relevant output in `prompt`.
5. Re-run the failed command, then the full trio. After the trio passes, run `graphify update .`.
