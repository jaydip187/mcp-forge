# syntax=docker/dockerfile:1
#
# docker build -t mcp-forge .
# docker run -p 7410:7410 -v mcp-forge-data:/data -e MCP_FORGE_ADMIN_TOKEN=... mcp-forge

# 1. The web UI, embedded into the binary at compile time.
FROM node:22-bookworm-slim AS ui
WORKDIR /src/ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci
COPY ui/ ./
RUN npm run build

# 2. The binary.
FROM rust:1.91-bookworm AS build
WORKDIR /src
COPY . .
COPY --from=ui /src/ui/dist ui/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p mcp-forge \
    && cp target/release/mcp-forge /usr/local/bin/mcp-forge

# 3. Runtime: no compilers, a non-root user, data in a volume.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin forge \
    && mkdir -p /data \
    && chown forge:forge /data
COPY --from=build /usr/local/bin/mcp-forge /usr/local/bin/mcp-forge
USER forge
# Listening beyond loopback makes mcp-forge require MCP_FORGE_ADMIN_TOKEN.
ENV MCP_FORGE_BIND=0.0.0.0:7410 \
    MCP_FORGE_DATA_DIR=/data
VOLUME ["/data"]
EXPOSE 7410
ENTRYPOINT ["mcp-forge"]
CMD ["serve"]
