# syntax=docker/dockerfile:1.7
FROM --platform=linux/amd64 rust:1.97.1-bookworm AS builder
WORKDIR /src/notes-app
# Keep the workspace's existing ../BubblePaw-base-agent path dependencies.
COPY --from=bubblepaw Cargo.toml /src/BubblePaw-base-agent/Cargo.toml
COPY --from=bubblepaw crates/bubblepaw-core /src/BubblePaw-base-agent/crates/bubblepaw-core
COPY --from=bubblepaw crates/bubblepaw-runner /src/BubblePaw-base-agent/crates/bubblepaw-runner
COPY --from=bubblepaw crates/bubblepaw-mcp /src/BubblePaw-base-agent/crates/bubblepaw-mcp
COPY . .
RUN --mount=type=cache,id=notes-hub-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=notes-hub-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=notes-hub-target-amd64,target=/src/notes-app/target,sharing=locked \
    cargo build --release -p notes_hub --locked \
    && install -Dm755 target/release/notes-hub /out/notes-hub

FROM --platform=linux/amd64 node:22.18.0-bookworm-slim AS node
FROM --platform=linux/amd64 oven/bun:1.4.2 AS bun
FROM --platform=linux/amd64 debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git tini curl libstdc++6 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
RUN npm install --global @anthropic-ai/claude-code@2.1.293 \
    && npm cache clean --force
WORKDIR /opt/bubblepaw/claude-code
COPY --from=bubblepaw extensions/claude-code/package.json extensions/claude-code/bun.lock extensions/claude-code/NOTICE ./
COPY --from=bubblepaw extensions/claude-code/src ./src
# Build contexts can carry 0600 files; the hub user must read the extension.
RUN chmod -R a+rX /opt/bubblepaw
RUN bun install --production --frozen-lockfile
COPY --from=builder /out/notes-hub /usr/local/bin/notes-hub
RUN groupadd --gid 10001 hub \
    && useradd --uid 10001 --gid 10001 --home-dir /data/home --no-create-home hub \
    && mkdir -p /data/home/.claude \
    && touch /data/hub.toml \
    && chown -R hub:hub /data
ENV HOME=/data/home \
    NOTES_HUB_DATA_DIR=/data \
    NOTES_HUB_AGENT_BUN=/usr/local/bin/bun \
    NOTES_HUB_AGENT_CLAUDE_CODE_EXTENSION=/opt/bubblepaw/claude-code \
    DISABLE_AUTOUPDATER=1
WORKDIR /data
USER hub:hub
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "notes-hub", "--config", "/data/hub.toml"]
