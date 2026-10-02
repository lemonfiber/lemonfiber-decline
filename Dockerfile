# The decline service's image: built from this repository, distroless, non-root.
#
# The build stage compiles a static binary for the platform being built; the image
# holds that binary and nothing else, with no shell and no package manager
# (ADR-0033 §2). Its health check is the binary asking itself, since the image has
# no HTTP client to ask with.

FROM rust:1.97.1-alpine3.22@sha256:df4efa4e0cdfb5245fa06e3f431387b2bcc96782ce5681b7fb6b0297d745bc29 AS build
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
