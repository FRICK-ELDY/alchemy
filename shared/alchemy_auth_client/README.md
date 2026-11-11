# alchemy_auth_client

Shared auth-server JWKS cache + user JWT verify for Alchemy **world** / **assets** (x002f / P3-4 / **x008c**).

Path dependency from `world-server` and `assets-server` (`../shared/alchemy_auth_client`).
Not published to Hex.

Client IP / trusted proxies live in [`alchemy_client_ip`](../alchemy_client_ip/) (x004b) — not here.

**JWKS (x008c):** fetch failures stamp the same cooldown as successes; `signer_for_kid` miss path replies from cache only (async refresh, in-flight coalesced). Does not implement host soft/hard TTL.

```elixir
{AlchemyAuthClient.Jwks, otp_app: :world, name: World.Auth.Jwks}
AlchemyAuthClient.Verify.verify(token, otp_app: :world, jwks: World.Auth.Jwks)
```
