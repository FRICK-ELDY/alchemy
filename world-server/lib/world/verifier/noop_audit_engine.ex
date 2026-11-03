defmodule World.Verifier.NoopAuditEngine do
  @moduledoc """
  Default audit engine: always `:ok`.

  Proprietary Spot-Check heuristics replace this via `:world, :audit_engine`.
  """

  @behaviour World.Verifier.AuditEngine

  @impl true
  def spot_check(_room_id, _host_sub, _frame_bytes, _context), do: :ok
end
