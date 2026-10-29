# alchemy_client_ip

Trusted-proxy client IP (`CF-Connecting-IP` → `X-Forwarded-For`) for Alchemy
**auth** / **world** (x004b). Path dependency only — not on Hex.

```elixir
AlchemyClientIp.from_conn(conn, otp_app: :auth)
```

Does **not** live in `alchemy_auth_client` (JWKS/verify stays separate so assets
does not pull `remote_ip`).
