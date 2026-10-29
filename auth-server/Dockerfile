# Pin Elixir/Alpine versions so the release runtime matches the build image's libc/OpenSSL.
ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23

# Build context: repository root (shared/alchemy_client_ip path dep · x004b).
FROM ${ELIXIR_IMAGE} AS dev

# wget: hermetic compose healthcheck (x001f); not relying on busybox alone.
RUN apk add --no-cache build-base git wget

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

COPY shared/alchemy_client_ip /shared/alchemy_client_ip
COPY auth-server/mix.exs auth-server/mix.lock ./
RUN mix deps.get || true

COPY auth-server/config config
COPY auth-server/lib lib
COPY auth-server/priv priv
COPY auth-server/test test
COPY auth-server/.formatter.exs ./
COPY auth-server/AGENTS.md ./

RUN mix deps.get && mix compile

EXPOSE 4002

CMD ["mix", "phx.server"]

# Production release build.
FROM ${ELIXIR_IMAGE} AS build

RUN apk add --no-cache build-base git

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

ENV MIX_ENV=prod

COPY shared/alchemy_client_ip /shared/alchemy_client_ip
COPY auth-server/mix.exs auth-server/mix.lock ./
RUN mix deps.get --only prod

COPY auth-server/config config
COPY auth-server/lib lib
COPY auth-server/priv priv
COPY auth-server/.formatter.exs ./

RUN mix compile
RUN mix release

# Production runtime (mix release).
FROM alpine:${ALPINE_VERSION} AS release

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates

WORKDIR /app

RUN adduser -D -h /app auth && chown auth:auth /app

COPY --from=build --chown=auth:auth /app/_build/prod/rel/auth ./

USER auth

ENV PHX_SERVER=true
ENV PORT=4002

EXPOSE 4002

CMD ["bin/auth", "start"]
