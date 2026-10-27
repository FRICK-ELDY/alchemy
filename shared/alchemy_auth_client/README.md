# alchemy_auth_client

Shared auth-server JWKS cache + user JWT verify for Alchemy **world** / **assets** (x002f / P3-4).

Path dependency from `world-server` and `assets-server` (`../shared/alchemy_auth_client`).
Not published to Hex.

```elixir
{AlchemyAuthClient.Jwks, otp_app: :world, name: World.Auth.Jwks}
AlchemyAuthClient.Verify.verify(token, otp_app: :world, jwks: World.Auth.Jwks)
```
