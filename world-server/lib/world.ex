defmodule World do
  @moduledoc """
  world-server — Room Token issuer + persistent room ledger (x0028).

  Guarantees: Ash/SQLite room ledger (owner + DemoScene) + short-lived RS256 Room Tokens + JWKS.
  Non-guarantees: visibility ACL, verifier, S2S federation, AshPostgres swap (documented path).
  """
end
