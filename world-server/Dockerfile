ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23

FROM ${ELIXIR_IMAGE} AS dev

RUN apk add --no-cache build-base git

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

COPY mix.exs ./
RUN mix deps.get || true

COPY config config
COPY lib lib
COPY test test
COPY .formatter.exs ./

RUN mix deps.get && mix compile

EXPOSE 4003

CMD ["mix", "phx.server"]

FROM ${ELIXIR_IMAGE} AS build

RUN apk add --no-cache build-base git

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

ENV MIX_ENV=prod

COPY mix.exs ./
RUN mix deps.get --only prod

COPY config config
COPY lib lib
COPY .formatter.exs ./

RUN mix compile
RUN mix release

FROM alpine:${ALPINE_VERSION} AS release

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates

WORKDIR /app

RUN adduser -D -h /app world && chown world:world /app

COPY --from=build --chown=world:world /app/_build/prod/rel/world ./

USER world

ENV PHX_SERVER=true
ENV PORT=4003

EXPOSE 4003

CMD ["bin/world", "start"]
