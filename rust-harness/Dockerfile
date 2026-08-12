FROM rust:1.96-bookworm AS builder

WORKDIR /build/rust-harness
COPY rust-harness/Cargo.toml rust-harness/Cargo.lock ./
COPY rust-harness/src ./src
COPY rust-harness/prompts ./prompts
COPY rust-harness/characters ./characters
COPY personas /build/personas
RUN --mount=type=cache,id=agent-arena-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=agent-arena-cargo-target,target=/build/cargo-target,sharing=locked \
    CARGO_TARGET_DIR=/build/cargo-target cargo build --release --locked \
        --bin agent-arena-npc-harness \
        --bin migrate-mastra-memory \
        --bin tactical-probe \
        --bin tactical-replay \
    && install -Dm755 /build/cargo-target/release/agent-arena-npc-harness /build/out/agent-arena-npc-harness \
    && install -Dm755 /build/cargo-target/release/migrate-mastra-memory /build/out/migrate-mastra-memory \
    && install -Dm755 /build/cargo-target/release/tactical-probe /build/out/tactical-probe \
    && install -Dm755 /build/cargo-target/release/tactical-replay /build/out/tactical-replay

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10003 harness \
    && useradd --uid 10003 --gid 10003 --no-create-home --home-dir /nonexistent harness

WORKDIR /app
COPY --from=builder /build/out/agent-arena-npc-harness /usr/local/bin/agent-arena-npc-harness
COPY --from=builder /build/out/migrate-mastra-memory /usr/local/bin/migrate-mastra-memory
COPY --from=builder /build/out/tactical-probe /usr/local/bin/tactical-probe
COPY --from=builder /build/out/tactical-replay /usr/local/bin/tactical-replay
COPY --from=builder /build/rust-harness/characters /app/rust-harness/characters
COPY --from=builder /build/personas /app/personas

USER 10003:10003
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/agent-arena-npc-harness"]
