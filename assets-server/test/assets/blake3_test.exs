defmodule Assets.Blake3Test do
  use ExUnit.Case, async: false

  alias Assets.Blake3.Worker

  # Official BLAKE3 unkeyed test vectors (empty / "abc").
  @empty "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
  @abc "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"

  test "hash_hex empty and abc via permanent Port" do
    assert Assets.Blake3.hash_hex("") == @empty
    assert Assets.Blake3.hash_hex("abc") == @abc
  end

  test "hash_hex reuses pool for successive calls" do
    assert Assets.Blake3.hash_hex("abc") == @abc
    assert Assets.Blake3.hash_hex("") == @empty
    assert Assets.Blake3.hash_hex("abc") == @abc
  end

  test "hash_hex reopens Port after forced close (fail-closed then recover)" do
    pid = GenServer.whereis(Worker.via(0))
    %{port: old} = :sys.get_state(pid)
    assert is_port(old)
    Port.close(old)

    new_port = await_replaced_port(pid, old)
    assert is_port(new_port)
    assert new_port != old

    assert Assets.Blake3.hash_hex("abc") == @abc
    assert Assets.Blake3.hash_hex("") == @empty
  end

  test "pool has distinct partition Ports and concurrent hashes agree" do
    partitions = Application.fetch_env!(:assets, :blake3_hash_partitions)
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

    # Hit each partition explicitly (not only unique_integer luck).
    per_partition =
      Enum.map(0..(partitions - 1), fn i ->
        data = "part-#{i}"

        assert {:ok, hex} =
                 GenServer.call(Worker.via(i), {:hash, data}, 30_000)

        {i, data, hex}
      end)

    results =
      1..partitions
      |> Task.async_stream(
        fn i ->
          data = "pool-#{i}"
          {data, Assets.Blake3.hash_hex(data)}
        end,
        max_concurrency: partitions,
        timeout: 30_000
      )
      |> Enum.map(fn {:ok, pair} -> pair end)

    assert length(results) == partitions

    for {_i, data, hex} <- per_partition do
      assert hex == Assets.Blake3.hash_hex(data)
      assert Regex.match?(~r/^[0-9a-f]{64}$/, hex)
    end

    for {data, hex} <- results do
      assert hex == Assets.Blake3.hash_hex(data)
      assert Regex.match?(~r/^[0-9a-f]{64}$/, hex)
    end
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
