ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23

FROM ${ELIXIR_IMAGE} AS dev

RUN apk add --no-cache build-base git wget

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

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates su-exec \
 && adduser -D -h /app world \
 && mkdir -p /var/lib/world \
 && chown world:world /app /var/lib/world

WORKDIR /app

COPY --from=build --chown=world:world /app/_build/prod/rel/world ./
COPY docker-entrypoint.release.sh /docker-entrypoint.release.sh
RUN chmod 755 /docker-entrypoint.release.sh

# Entrypoint chowns the Room Token key volume, then drops to `world` (x001e).
USER root

ENV PHX_SERVER=true
ENV PORT=4003

EXPOSE 4003

ENTRYPOINT ["/docker-entrypoint.release.sh"]
CMD ["bin/world", "start"]
