ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
ARG ALPINE_VERSION=3.23
ARG RUST_IMAGE=rust:1-alpine

FROM ${RUST_IMAGE} AS b3sum
RUN apk add --no-cache musl-dev \
 && cargo install b3sum --locked \
 && cp /usr/local/cargo/bin/b3sum /b3sum

FROM ${ELIXIR_IMAGE} AS dev

RUN apk add --no-cache build-base git
COPY --from=b3sum /b3sum /usr/local/bin/b3sum

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

COPY mix.exs mix.lock ./
RUN mix deps.get || true

COPY config config
COPY lib lib
COPY priv priv
COPY test test
COPY .formatter.exs ./

RUN mix deps.get && mix compile

EXPOSE 4004

CMD ["mix", "phx.server"]

# Production release build (x001e).
FROM ${ELIXIR_IMAGE} AS build

RUN apk add --no-cache build-base git
COPY --from=b3sum /b3sum /usr/local/bin/b3sum

WORKDIR /app

RUN mix local.hex --force && mix local.rebar --force

ENV MIX_ENV=prod

COPY mix.exs mix.lock ./
RUN mix deps.get --only prod

COPY config config
COPY lib lib
COPY priv priv
COPY .formatter.exs ./

RUN mix compile
RUN mix release

FROM alpine:${ALPINE_VERSION} AS release

RUN apk add --no-cache libstdc++ openssl ncurses-libs ca-certificates su-exec \
 && adduser -D -h /app assets \
 && mkdir -p /data \
 && chown assets:assets /app /data

COPY --from=b3sum /b3sum /usr/local/bin/b3sum

WORKDIR /app

COPY --from=build --chown=assets:assets /app/_build/prod/rel/assets ./
COPY docker-entrypoint.release.sh /docker-entrypoint.release.sh
RUN chmod 755 /docker-entrypoint.release.sh

# Entrypoint repairs /data ownership then su-exec assets (x001e).
# Migrate: `docker compose --profile migrate run --rm assets-migrate` (not bare `exec`).
USER root

ENV PHX_SERVER=true
ENV PORT=4004

EXPOSE 4004

ENTRYPOINT ["/docker-entrypoint.release.sh"]
CMD ["bin/assets", "start"]
