defmodule AlchemyAuthClient do
  @moduledoc """
  Shared auth-server JWKS cache and user JWT verification (x002f / P3-4).

  Guarantees: one GenServer implementation for world/assets; unknown-kid fetch
  cooldown when the cache is non-empty.
  Non-guarantees: Hex release, host-server Rust JWKS, revocation lists.
  """
end
