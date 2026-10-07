# The decline service's image: built from this repository, distroless, non-root.
#
# The build stage compiles a static binary for the platform being built; the image
# holds that binary and nothing else, with no shell and no package manager
# (ADR-0033 §2). Its health check is the binary asking itself, since the image has
# no HTTP client to ask with.

FROM rust:1.98.1-alpine3.22@sha256:a1796ca6fa216d6727b5f61c69e4c665b120b1a4dcb969639e2f25f1ed309456 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked && cp target/release/decline /decline

FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
COPY --from=build /decline /decline
USER nonroot:nonroot
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 CMD ["/decline", "health"]
ENTRYPOINT ["/decline"]
