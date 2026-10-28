ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23
ARG RUST_IMAGE=rust:1-alpine

# x0039: canonical ManifestSeal hash (same encode path as peers).
FROM ${RUST_IMAGE} AS seal-hash
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY protocol /src/protocol
WORKDIR /src/protocol
RUN cargo build --release --bin manifest-seal-hash \
 && cp target/release/manifest-seal-hash /manifest-seal-hash

# Build context: repository root (shared/alchemy_auth_client path dep).
FROM ${ELIXIR_IMAGE} AS dev

RUN apk add --no-cache build-base git wget
COPY --from=seal-hash /manifest-seal-hash /usr/local/bin/manifest-seal-hash

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

COPY shared/alchemy_auth_client /shared/alchemy_auth_client
COPY world-server/mix.exs world-server/mix.lock ./
RUN mix deps.get || true

COPY world-server/config config
COPY world-server/lib lib
COPY world-server/priv priv
COPY world-server/test test
COPY world-server/.formatter.exs ./

RUN mix deps.get && mix compile

EXPOSE 4003

CMD ["mix", "phx.server"]

FROM ${ELIXIR_IMAGE} AS build

RUN apk add --no-cache build-base git
COPY --from=seal-hash /manifest-seal-hash /usr/local/bin/manifest-seal-hash

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

ENV MIX_ENV=prod

COPY shared/alchemy_auth_client /shared/alchemy_auth_client
COPY world-server/mix.exs world-server/mix.lock ./
RUN mix deps.get --only prod

COPY world-server/config config
COPY world-server/lib lib
COPY world-server/priv priv
COPY world-server/.formatter.exs ./

RUN mix compile
RUN mix release

FROM alpine:${ALPINE_VERSION} AS release

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates su-exec \
 && adduser -D -h /app world \
 && mkdir -p /var/lib/world \
 && chown world:world /app /var/lib/world

WORKDIR /app

COPY --from=seal-hash /manifest-seal-hash /usr/local/bin/manifest-seal-hash
COPY --from=build --chown=world:world /app/_build/prod/rel/world ./
COPY world-server/docker-entrypoint.release.sh /docker-entrypoint.release.sh
RUN chmod 755 /docker-entrypoint.release.sh

# Entrypoint chowns the Room Token key volume, then drops to `world` (x001e).
USER root

ENV PHX_SERVER=true
ENV PORT=4003

EXPOSE 4003

ENTRYPOINT ["/docker-entrypoint.release.sh"]
CMD ["bin/world", "start"]
