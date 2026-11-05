ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23
ARG RUST_IMAGE=rust:1-alpine

# x0075: permanent blake3-hash Port worker (replaces b3sum).
FROM ${RUST_IMAGE} AS blake3-hash
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY protocol /src/protocol
WORKDIR /src/protocol
RUN cargo build --release --bin blake3-hash \
 && cp target/release/blake3-hash /blake3-hash

# Build context: repository root (shared/alchemy_auth_client path dep).
FROM ${ELIXIR_IMAGE} AS dev

RUN apk add --no-cache build-base git wget
COPY --from=blake3-hash /blake3-hash /usr/local/bin/blake3-hash

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

COPY shared/alchemy_auth_client /shared/alchemy_auth_client
COPY assets-server/mix.exs assets-server/mix.lock ./
RUN mix deps.get || true

COPY assets-server/config config
COPY assets-server/lib lib
COPY assets-server/priv priv
COPY assets-server/test test
COPY assets-server/.formatter.exs ./

RUN mix deps.get && mix compile

EXPOSE 4004

CMD ["mix", "phx.server"]

# Production release build (x001e).
FROM ${ELIXIR_IMAGE} AS build

RUN apk add --no-cache build-base git
COPY --from=blake3-hash /blake3-hash /usr/local/bin/blake3-hash

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

ENV MIX_ENV=prod

COPY shared/alchemy_auth_client /shared/alchemy_auth_client
COPY assets-server/mix.exs assets-server/mix.lock ./
RUN mix deps.get --only prod

COPY assets-server/config config
COPY assets-server/lib lib
COPY assets-server/priv priv
COPY assets-server/.formatter.exs ./

RUN mix compile
RUN mix release

FROM alpine:${ALPINE_VERSION} AS release

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates su-exec \
 && adduser -D -h /app assets \
 && mkdir -p /data \
 && chown assets:assets /app /data

COPY --from=blake3-hash /blake3-hash /usr/local/bin/blake3-hash

WORKDIR /app

COPY --from=build --chown=assets:assets /app/_build/prod/rel/assets ./
COPY assets-server/docker-entrypoint.release.sh /docker-entrypoint.release.sh
RUN chmod 755 /docker-entrypoint.release.sh

# Entrypoint repairs /data ownership then su-exec assets (x001e).
# Migrate: `docker compose --profile migrate run --rm assets-migrate` (not bare `exec`).
USER root

ENV PHX_SERVER=true
ENV PORT=4004

EXPOSE 4004

ENTRYPOINT ["/docker-entrypoint.release.sh"]
CMD ["bin/assets", "start"]
