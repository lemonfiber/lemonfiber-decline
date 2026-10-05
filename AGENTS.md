# AGENTS.md — lemonfiber-decline

> **Start at the roadmap and board on [lemonfiber.app](https://lemonfiber.app),
> rendered from the report of where every unreleased version stands. Then the
> rules** every repository shares:
> [working in the repositories](https://github.com/lemonfiber/spec/blob/main/50-governance/working-in-the-repositories.md)
> and [the rules for agents](https://github.com/lemonfiber/spec/blob/main/50-governance/ai-contributors.md).
> This file holds only what is true of this repository.

## What this repo is

The decline service: the one image that answers an invitation's decline address (ADR-0029). One Rust binary crate and one distroless image. See the spec:
[`30-repos/lemonfiber-decline.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-decline.md).

## The one rule you cannot break here

**It acts only on what the core's invitation table names.** It takes the table and its Jellyfin key over `invite.decline`, on a port of its own that the household never reaches, under the plugin's key. It makes fixed Jellyfin calls (ADR-0029 §2b) and forwards nothing. Its whole effect on an account the table names is one of two: `IsDisabled` from false to true, on a refusal or when an invitation's window closes, or the account removed, when its window closes and every guard in ADR-0029 §2a holds on reads made just before the call. Nothing is removed on doubt, and nothing is left open on it: where whether an account was claimed cannot be read, or the record of lapses cannot be read, it is switched off and kept. A route that passes a request through to Jellyfin, an account the table does not name being touched, or a removal without every guard holding, is a defect however it is reached.

## Code standards (enforced)

- `unsafe` is **forbidden**. No `unwrap`/`expect`/`panic`/`todo` in non-test code.
- **No lint suppressions in `src/`**: change the code or the rule, never `#[allow]`.
- The image stays distroless and non-root, with nothing in it but the binary.
