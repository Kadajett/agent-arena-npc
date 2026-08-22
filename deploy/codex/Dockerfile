FROM node:22-bookworm-slim

ARG CODEX_VERSION=0.149.0

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN npm install --global "@openai/codex@${CODEX_VERSION}" \
    && npm cache clean --force

RUN groupadd --gid 10003 zella \
    && useradd --uid 10003 --gid 10003 --create-home --home-dir /home/zella zella \
    && mkdir -p /opt/zella /workspace \
    && chown -R zella:zella /opt/zella /workspace

COPY --chown=zella:zella personas/zella.md /opt/zella/zella.md
COPY --chown=zella:zella personas/zella-world.md /opt/zella/zella-world.md
COPY --chown=zella:zella deploy/codex/run.sh /opt/zella/run.sh
COPY --chown=zella:zella deploy/codex/tick.md /opt/zella/tick.md

USER 10003:10003
WORKDIR /workspace
ENTRYPOINT ["/opt/zella/run.sh"]
