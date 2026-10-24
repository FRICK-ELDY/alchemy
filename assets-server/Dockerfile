ARG ELIXIR_IMAGE=elixir:1.19-otp-28-alpine
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
