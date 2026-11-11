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
    pid = GenServer.whereis(Worker.via(0))
    %{port: old} = :sys.get_state(pid)
    assert is_port(old)
    Port.close(old)

    new_port = await_replaced_port(pid, old)
    assert is_port(new_port)
    assert new_port != old

    # Soft error still works on the replacement Port.
    assert {:error, {:cli_failed, 1, _}} = ManifestSealHash.blake3_hex(%{})
  end

  test "pool has distinct partition Ports and concurrent soft-errors" do
    partitions = Application.fetch_env!(:world, :manifest_seal_hash_partitions)
    assert PartitionSupervisor.partitions(Worker.partitioner_name()) == partitions
    assert partitions >= 2

    pids = Enum.map(0..(partitions - 1), &GenServer.whereis(Worker.via(&1)))

    ports =
      Enum.map(pids, fn pid ->
        assert is_pid(pid)
        %{port: port} = :sys.get_state(pid)
        assert is_port(port)
        port
      end)

    assert length(Enum.uniq(pids)) == partitions
    assert length(Enum.uniq(ports)) == partitions

    # Explicit per-partition calls (soft error keeps each Port alive).
    for i <- 0..(partitions - 1) do
      assert {:error, {:cli_failed, 1, _}} =
               GenServer.call(Worker.via(i), {:hash, "{}"}, 30_000)
    end

    results =
      1..partitions
      |> Task.async_stream(
        fn i ->
          ManifestSealHash.blake3_hex(%{still: "bad-#{i}"})
        end,
        max_concurrency: partitions,
        timeout: 30_000
      )
      |> Enum.map(fn {:ok, result} -> result end)

    assert length(results) == partitions
    assert Enum.all?(results, &match?({:error, {:cli_failed, 1, _}}, &1))
  end

  defp await_replaced_port(pid, old, attempts \\ 50) do
    Enum.find_value(1..attempts, fn _ ->
      Process.sleep(20)

      case :sys.get_state(pid) do
        %{port: port} when is_port(port) and port != old -> port
        _ -> nil
      end
    end) ||
      flunk("Port was not replaced after close (old=#{inspect(old)})")
  end
end
