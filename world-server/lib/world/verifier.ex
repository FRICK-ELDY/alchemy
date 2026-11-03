defmodule World.Verifier do
  @moduledoc """
  Host audit façade for Spot-Check injection.

  Guarantees: resolves `:world, :audit_engine` (default `NoopAuditEngine`) so
  proprietary engines can be injected without editing this OSS tree.
  Non-guarantees: no Spot-Check orchestration caller is wired yet — signature /
  liveliness / failover paths do not invoke `spot_check/4` today; this module
  is the reserved injection point for a future auditor. Detailed heuristics
  stay behind `World.Verifier.AuditEngine` in private plugins.
  """

  @spec spot_check(String.t(), String.t(), binary(), map()) :: World.Verifier.AuditEngine.result()
  def spot_check(room_id, host_sub, frame_bytes, context \\ %{})
      when is_binary(room_id) and is_binary(host_sub) and is_binary(frame_bytes) and
             is_map(context) do
    engine().spot_check(room_id, host_sub, frame_bytes, context)
  end

  defp engine do
    Application.get_env(:world, :audit_engine, World.Verifier.NoopAuditEngine)
  end
end
