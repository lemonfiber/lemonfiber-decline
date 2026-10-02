# lemonfiber-decline

The decline service: the one image that answers an invitation's decline address. It serves a page naming what would be declined, takes the refusal, and disables the invited account through three fixed Jellyfin calls, with the one API key minted for it alone.

It is the source of `ghcr.io/lemonfiber/decline`, which [`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack) runs as the service `decline`. Its decisions are [ADR-0029](https://github.com/lemonfiber/spec/blob/main/00-overview/decisions/) and [ADR-0033](https://github.com/lemonfiber/spec/blob/main/00-overview/decisions/0033-each-image-lemonfiber-builds-for-the-stack-has-its-own-repository.md); what it holds is in the specification's [`30-repos/lemonfiber-decline.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-decline.md).

It is released on lemonfiber's version train, tagged before the core, and the stack pins the digest each tag publishes.

## Licence

[Hippocratic License 3.0](LICENSE), as every repository of lemonfiber's.
