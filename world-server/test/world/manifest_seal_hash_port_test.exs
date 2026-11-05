defmodule World.ManifestSealHashPortTest do
  use ExUnit.Case, async: false

  alias World.ManifestSealHash
  alias World.ManifestSealHash.Worker

  test "soft error on invalid JSON keeps Port alive for next call" do
    assert {:error, {:cli_failed, 1, reason}} = ManifestSealHash.blake3_hex(%{bad: true})
    assert is_binary(reason)

    # Reuse after soft error (Port must still be serving).
    assert {:error, {:cli_failed, 1, _}} = ManifestSealHash.blake3_hex(%{still: "bad"})
  end

  test "reopens Port after forced close" do
    %{port: old} = :sys.get_state(Worker)
    assert is_port(old)
    Port.close(old)

    new_port = await_replaced_port(Worker, old)
    assert is_port(new_port)
    assert new_port != old

    # Soft error still works on the replacement Port.
    assert {:error, {:cli_failed, 1, _}} = ManifestSealHash.blake3_hex(%{})
  end

  defp await_replaced_port(name, old, attempts \\ 50) do
    Enum.find_value(1..attempts, fn _ ->
      Process.sleep(20)

      case :sys.get_state(name) do
        %{port: port} when is_port(port) and port != old -> port
        _ -> nil
      end
    end) ||
      flunk("Port was not replaced after close (old=#{inspect(old)})")
  end
end
