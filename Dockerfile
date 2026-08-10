# One image, one character per container. NPC_CHARACTER decides who it is.
FROM node:24-slim AS build

WORKDIR /build
COPY package.json package-lock.json* ./
RUN npm ci --no-audit --no-fund
COPY tsconfig.json ./
COPY src ./src
RUN npx tsc -p tsconfig.json

FROM node:24-slim

RUN useradd --uid 10003 --create-home --shell /usr/sbin/nologin npc

WORKDIR /npc
COPY package.json package-lock.json* ./
RUN npm ci --omit=dev --no-audit --no-fund && npm cache clean --force
COPY --from=build /build/dist ./dist
COPY personas ./personas

# Memory lives on a volume: a character who forgets everyone between deploys is
# worse than one with no memory at all. The directory has to exist in the image
# and be owned by the runtime user, because a fresh named volume takes its
# ownership from the image. Without this the container gets a root-owned mount
# it cannot write to, and SQLite fails to open the database.
RUN mkdir -p /npc/var && chown -R 10003:10003 /npc/var
ENV NPC_MEMORY_DIR=/npc/var
ENV NODE_ENV=production

USER 10003
CMD ["node", "dist/index.js"]
