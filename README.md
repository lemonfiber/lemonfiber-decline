# lemonfiber-decline

The decline service is a small web service inside a [lemonfiber](https://github.com/lemonfiber/lemonfiber)
media stack. It lets somebody who was invited to the household's media server
say no, and closes the account that was made for them. Once a minute it also
takes back every invitation whose window has closed: it removes an account
nobody was ever seen in, and switches off one somebody has been in.

This repository builds the container image `ghcr.io/lemonfiber/decline`. You do
not run it yourself: lemonfiber installs it as the plugin `decline`, from the
`plugin.toml` here, and writes its configuration.

## Why it exists

When the operator invites somebody, lemonfiber creates a Jellyfin account for
them and makes an invitation. The invitation carries two addresses: the
household's front door, where the invitee signs in, and one to decline it. The
decline address must work without the operator being around, and without giving
the invitee, or anyone who finds the link, any other power over Jellyfin.

So the decline address is served by this one small service, which holds a
Jellyfin API key made for it alone and uses it for fixed calls only, each about
one account an invitation names.

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
| Listens on | Port 8080 inside its container for the decline page, which the stack publishes to the household, and port 8081 for lemonfiber's `invite.decline` contract, published on loopback alone |
| Configuration | `/config`: the invitations, the Jellyfin key and the SHA-256 of each certificate Jellyfin's internal TLS port may present, as lemonfiber hands them to the service (`invitations.json`, `jellyfin.key`, `media-server.pins`), the key lemonfiber asks the contract with (`lemonfiber.key`), and the `refusals.json` and `lapses.json` the service writes, owner-only, and answers over `invite.decline` |
| Settings | `LEMONFIBER_DECLINE_CONFIG` moves the configuration directory; `LEMONFIBER_DECLINE_JELLYFIN` points at Jellyfin's internal TLS port (default `https://jellyfin:8920`), and the service refuses to start where it names anything but an HTTPS address; the service sends its key only over TLS to a certificate whose SHA-256 is pinned, and reaches Jellyfin not at all while none is |
| Health check | `decline health`, which asks the running service over loopback |
| Container | Distroless base, non-root user, read-only root filesystem, no kernel capabilities |

lemonfiber writes the compose entry and its networks from `plugin.toml`.
The design is written up in the specification:
[`30-repos/lemonfiber-decline.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-decline.md).

## Building and testing

You need Rust (the version in [`rust-toolchain.toml`](rust-toolchain.toml)) and
Docker. The build fetches the contract, the adapter kit, the ports and the file
formats from the [`lemonfiber`](https://github.com/lemonfiber/lemonfiber)
repository, at the commit pinned in [`Cargo.toml`](Cargo.toml), so it needs network access the
first time.

```sh
cargo test
docker build -t decline .
```

Images are published from version tags. Each tag builds `linux/amd64` and
`linux/arm64`, and `plugin.toml` pins the digest it published.

## Contributing and security

Read the [contributing guide](https://github.com/lemonfiber/spec/blob/main/50-governance/contributing.md)
before opening a pull request. Report a vulnerability as the
[security policy](https://github.com/lemonfiber/.github/blob/main/SECURITY.md)
describes, not in a public issue.

## Licence

[Hippocratic License 3.0](LICENSE).
