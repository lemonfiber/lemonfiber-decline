# AGENTS.md — lemonfiber-decline

Guidance for any AI agent (Cursor, Codex, Aider, Claude Code, …) working in this
repo.

> **Start at the report** of where every unreleased version stands: the summary
> of the newest run of the spec's [`state` workflow](https://github.com/lemonfiber/spec/actions/workflows/state.yml),
> or `just goals <version>` in a spec checkout.
> **Then the rules every repository shares:**
> [working in the repositories](https://github.com/lemonfiber/spec/blob/main/50-governance/working-in-the-repositories.md)
> and [the rules for agents](https://github.com/lemonfiber/spec/blob/main/50-governance/ai-contributors.md).
> This file holds only what is true of `lemonfiber-decline`.

## What this repo is

The decline service: the one image that answers an invitation's decline address (ADR-0029). One Rust binary crate and one distroless image. See the spec:
[`30-repos/lemonfiber-decline.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-decline.md).

## The one rule you cannot break here

**It acts only on what the core's invitation table names.** It makes three fixed Jellyfin calls, for the one account a valid token names, changes only `IsDisabled` from false to true, and forwards nothing. A route that passes a request through to Jellyfin, or acts on an account the table does not name, is a defect however it is reached.

## Code standards (enforced)

- `unsafe` is **forbidden**. No `unwrap`/`expect`/`panic`/`todo` in non-test code.
- **No lint suppressions in `src/`**: change the code or the rule, never `#[allow]`.
- Every commit is signed and cites the requirement it serves (`Spec: <ID>`).
- The image stays distroless and non-root, with nothing in it but the binary.
