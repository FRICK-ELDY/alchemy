# alchemy_rate_limit

ETS fixed-window rate limiter for Alchemy **auth** / **world** (x0071).
Path dependency only — not on Hex.

```elixir
{:ok, _} =
  AlchemyRateLimit.start_link(
    name: MyApp.RateLimit,
    table: :my_app_rate_limit,
    otp_app: :my_app,
    config_key: MyApp.RateLimit,
    telemetry_prefix: [:my_app, :rate_limit],
    buckets: [login_ip: {:login, :ip}]
  )

AlchemyRateLimit.hit(MyApp.RateLimit, :login_ip, "127.0.0.1", %{
  limit: 30,
  period_ms: 60_000
})
```

Config (`Application.get_env(otp_app, config_key)`):

- `:enabled` — default `true`
- `:limits` — nested map used by cleanup (`action → axis → %{period_ms: …}`)
