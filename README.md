# lemonfiber-decline

The decline service is a small web service inside a [lemonfiber](https://github.com/lemonfiber/lemonfiber)
media stack. It lets somebody who was invited to the household's media server
say no, and closes the account that was made for them.

This repository builds the container image `ghcr.io/lemonfiber/decline`. You do
not run it yourself: [`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack)
runs it as the service `decline`, and `lemonfiber` writes its configuration.

## Why it exists

When the operator invites somebody, lemonfiber creates a Jellyfin account for
them and makes an invitation. The invitation carries two addresses: the
household's front door, where the invitee signs in, and one to decline it. The
decline address must work without the operator being around, and without giving
the invitee, or anyone who finds the link, any other power over Jellyfin.

So the decline address is served by this one small service, which holds a
Jellyfin API key made for it alone and uses it for three calls only, all about
the one account the invitation names.

## What it does

Opening an invitation's decline address (`/decline/<token>`) shows a page that
names the account the invitation was made for and offers one button, **Decline the invitation**. Pressing
it:

1. reads the account the invitation was made for;
2. checks the invitee has not already accepted it, by asking whether the account's
   password changed since the invitation was sent;
3. writes the account's policy back as disabled, so it can no longer be signed in
   to;
4. records the refusal, which lemonfiber reads to tell the operator.

An invitation that was accepted, already declined or is no longer open gets a
page saying so, and nothing changes. It never disables an administrator account.
One network address may ask to decline five times a minute.

## How the stack runs it

| | |
| --- | --- |
| Listens on | Port 8080 inside its container; the stack publishes it to the household |
| Configuration | `/config`, where `lemonfiber` writes `invitations.json` and `jellyfin.key`, and the service writes `refusals.json` |
| Settings | `LEMONFIBER_DECLINE_CONFIG` moves the configuration directory; `LEMONFIBER_DECLINE_JELLYFIN` points at Jellyfin (default `http://jellyfin:8096`) |
| Health check | `decline health`, which asks the running service over loopback |
| Container | Distroless base, non-root user, read-only root filesystem, no kernel capabilities |

The compose entry and its networks are in
[`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack).
The design is written up in the specification:
[`30-repos/lemonfiber-decline.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-decline.md).

## Building and testing

You need Rust (the version in [`rust-toolchain.toml`](rust-toolchain.toml)) and
Docker. The build fetches one crate from the
[`lemonfiber`](https://github.com/lemonfiber/lemonfiber) repository, at the
commit pinned in [`Cargo.toml`](Cargo.toml), so it needs network access the
first time.

```sh
cargo test
cargo clippy --all-targets --locked -- -D warnings
docker build -t decline .
```

Images are published from version tags. Each tag builds `linux/amd64` and
`linux/arm64`, and `lemonfiber-media-stack` pins the digest it published.

## Contributing and security

Read the [contributing guide](https://github.com/lemonfiber/spec/blob/main/50-governance/contributing.md)
before opening a pull request. Report a vulnerability as the
[security policy](https://github.com/lemonfiber/.github/blob/main/SECURITY.md)
describes, not in a public issue.

## Licence

[Hippocratic License 3.0](LICENSE).
