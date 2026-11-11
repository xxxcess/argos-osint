# syntax=docker/dockerfile:1

ARG NODE_IMAGE=node:24.22.1-bookworm-slim

# The upstream runtime has no shell. Add only the static applet binary used
# to load the root credential file before exec'ing SurrealDB.
FROM busybox:1.37.0-uclibc AS secret_tools
FROM surrealdb/surrealdb:v3.3.4@sha256:649a60a3b9c0177d9923c938e1c2563e48cde2a9d06ac2c0519714be719b02af AS surrealdb
COPY --from=secret_tools /bin/busybox /busybox
COPY --chmod=0555 deploy/start-surreal.sh /usr/local/bin/start-surreal
ENTRYPOINT ["/busybox", "sh", "/usr/local/bin/start-surreal"]

FROM ${NODE_IMAGE} AS web_source
RUN npm install --global pnpm@12.15.0
WORKDIR /opt/fleetworks-web
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.json ./
COPY patches ./patches
COPY apps/web-desktop ./apps/web-desktop
COPY packages ./packages

FROM web_source AS web_deps
ARG TARGETARCH
ENV NEXT_TELEMETRY_DISABLED=1
RUN --mount=type=cache,id=fleetworks-pnpm-${TARGETARCH},target=/pnpm/store,sharing=locked \
    pnpm install --frozen-lockfile --store-dir /pnpm/store

FROM web_deps AS web_build
RUN pnpm --dir apps/web-desktop build \
    && rm -rf apps/web-desktop/.next/cache \
    && mkdir apps/web-desktop/.next/cache

FROM web_deps AS web_dev
ARG FLEETWORKS_UID
ARG FLEETWORKS_GID
# Next updates its TypeScript setup and writes generated files in the app directory.
RUN mkdir -p apps/web-desktop/.next/cache \
    && chown "${FLEETWORKS_UID}:${FLEETWORKS_GID}" \
       apps/web-desktop apps/web-desktop/.next \
       apps/web-desktop/tsconfig.json apps/web-desktop/AGENTS.md
COPY deploy/check-web.mjs /usr/local/lib/fleetworks/check-web.mjs
WORKDIR /opt/fleetworks-web/apps/web-desktop
USER node
ENTRYPOINT ["node", "server.mjs", "--dev"]

FROM web_source AS web_prod_deps
ARG TARGETARCH
RUN --mount=type=cache,id=fleetworks-pnpm-${TARGETARCH},target=/pnpm/store,sharing=locked \
    pnpm install --prod --frozen-lockfile --store-dir /pnpm/store

FROM ${NODE_IMAGE} AS web
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1
COPY --from=web_prod_deps /opt/fleetworks-web /opt/fleetworks-web
COPY --from=web_build /opt/fleetworks-web/apps/web-desktop/.next /opt/fleetworks-web/apps/web-desktop/.next
COPY deploy/check-web.mjs /usr/local/lib/fleetworks/check-web.mjs
WORKDIR /opt/fleetworks-web/apps/web-desktop
USER node
ENTRYPOINT ["node", "server.mjs"]

FROM rust:1.95.0-bookworm AS build
ARG TARGETARCH
ENV CARGO_INCREMENTAL=0
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY apps/fleetworks-cli ./apps/fleetworks-cli
COPY apps/venture ./apps/venture
RUN --mount=type=cache,id=fleetworks-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=fleetworks-target-${TARGETARCH},target=/src/target,sharing=locked \
    cargo build --release --locked -p argos-api-host --no-default-features --features server,nlp,classifier \
    && install -D target/release/argos-api-host /out/argos-api-host

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libgcc-s1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/argos-api-host /usr/local/bin/argos-api-host
COPY --chmod=0555 deploy/check-api.sh /usr/local/bin/check-fleetworks-api
WORKDIR /workspace
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/argos-api-host"]

# One server compile for Venture and the six CLIs. Each final target is a
# separate image so a development cycle can build only the binaries it changed.
FROM rust:1.95.0-bookworm AS clients_build
ARG TARGETARCH
ENV CARGO_INCREMENTAL=0
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY apps/fleetworks-cli ./apps/fleetworks-cli
COPY apps/venture ./apps/venture
RUN --mount=type=cache,id=fleetworks-clients-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=fleetworks-clients-target-${TARGETARCH},target=/src/target,sharing=locked \
    cargo build --release --locked --bins -p fleetworks-cli -p fleetworks-venture \
    && mkdir -p /out \
    && for binary in venture fleetworks-atlas fleetworks-brain fleetworks-jobs fleetworks-tools fleetworks-models fleetworks-profile; do \
         install -D "target/release/${binary}" "/out/${binary}"; \
       done

FROM debian:bookworm-slim AS client_base
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
USER 65532:65532

FROM client_base AS venture
COPY --from=clients_build --chmod=0555 /out/venture /usr/local/bin/venture
ENTRYPOINT ["/usr/local/bin/venture"]

FROM client_base AS fleetworks-atlas
COPY --from=clients_build --chmod=0555 /out/fleetworks-atlas /usr/local/bin/fleetworks-atlas
ENTRYPOINT ["/usr/local/bin/fleetworks-atlas"]

FROM client_base AS fleetworks-brain
COPY --from=clients_build --chmod=0555 /out/fleetworks-brain /usr/local/bin/fleetworks-brain
ENTRYPOINT ["/usr/local/bin/fleetworks-brain"]

FROM client_base AS fleetworks-jobs
COPY --from=clients_build --chmod=0555 /out/fleetworks-jobs /usr/local/bin/fleetworks-jobs
ENTRYPOINT ["/usr/local/bin/fleetworks-jobs"]

FROM client_base AS fleetworks-tools
COPY --from=clients_build --chmod=0555 /out/fleetworks-tools /usr/local/bin/fleetworks-tools
ENTRYPOINT ["/usr/local/bin/fleetworks-tools"]

FROM client_base AS fleetworks-models
COPY --from=clients_build --chmod=0555 /out/fleetworks-models /usr/local/bin/fleetworks-models
ENTRYPOINT ["/usr/local/bin/fleetworks-models"]

FROM client_base AS fleetworks-profile
COPY --from=clients_build --chmod=0555 /out/fleetworks-profile /usr/local/bin/fleetworks-profile
ENTRYPOINT ["/usr/local/bin/fleetworks-profile"]
