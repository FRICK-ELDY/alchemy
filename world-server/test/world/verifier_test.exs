defmodule World.VerifierTest do
  use ExUnit.Case, async: false

  alias World.Verifier
  alias World.Verifier.NoopAuditEngine

  defmodule SuspectEngine do
    @behaviour World.Verifier.AuditEngine

    @impl true
    def spot_check(_room_id, _host_sub, _frame, _ctx), do: {:suspect, :fixture}
  end

  setup do
    previous = Application.get_env(:world, :audit_engine)
    Application.put_env(:world, :audit_engine, NoopAuditEngine)

    on_exit(fn ->
      if previous do
        Application.put_env(:world, :audit_engine, previous)
      else
        Application.delete_env(:world, :audit_engine)
      end
    end)

    :ok
  end

  test "default NoopAuditEngine always returns :ok" do
    assert :ok = Verifier.spot_check("room", "host-sub", <<1, 2, 3>>, %{generation: 1})
  end

  test "configured audit_engine is injected through Verifier façade" do
    Application.put_env(:world, :audit_engine, SuspectEngine)

    assert {:suspect, :fixture} =
             Verifier.spot_check("room", "host-sub", <<9>>, %{})
  end
end
