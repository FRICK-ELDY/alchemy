defmodule World.Network.Zenoh.ClientTest do
  use ExUnit.Case, async: false

  alias World.Network.Zenoh.Adapter
  alias World.Network.Zenoh.Client
  alias World.Network.Zenoh.LocalStubAdapter
  alias World.Network.Zenoh.ZenohexAdapter

  @key "alchemy/dev/room/local/testcap/manifest"

  test "adapter sample_message is the only delivery contract" do
    assert {:zenoh_sample, @key, <<"seal">>} =
             Adapter.sample_message(@key, <<"seal">>)
  end

  test "ZenohexAdapter normalizes %Zenohex.Sample{} into the shared contract" do
    sample = %Zenohex.Sample{
      key_expr: @key,
      payload: <<"from-nif">>,
      kind: :put
    }

    assert {:zenoh_sample, @key, <<"from-nif">>} =
             ZenohexAdapter.normalize_sample(sample)
  end

  test "publish delivers Adapter.sample_message to exact-match subscriber" do
    assert Application.get_env(:world, :zenoh_adapter) == LocalStubAdapter

    assert {:ok, sub_id} = Client.subscribe(@key, self())
    assert :ok = Client.publish(@key, <<"seal-bytes">>)

    assert_receive {:zenoh_sample, @key, <<"seal-bytes">>}, 1_000

    assert Adapter.sample_message(@key, <<"seal-bytes">>) ==
             {:zenoh_sample, @key, <<"seal-bytes">>}

    assert :ok = Client.unsubscribe(sub_id)
    assert :ok = Client.publish(@key, <<"after-unsub">>)
    refute_receive {:zenoh_sample, @key, <<"after-unsub">>}, 200
  end

  test "publish to a different key_expr does not notify subscriber" do
    assert {:ok, sub_id} = Client.subscribe(@key, self())
    assert :ok = Client.publish(@key <> "/other", <<"nope">>)
    refute_receive {:zenoh_sample, _, _}, 200
    assert :ok = Client.unsubscribe(sub_id)
  end

  test "dead subscriber pid is dropped from LocalStubAdapter subscriptions" do
    {:ok, pid} =
      Task.start(fn ->
        receive do
          :block -> :ok
        end
      end)

    assert {:ok, _sub_id} = Client.subscribe(@key, pid)
    assert map_size(:sys.get_state(LocalStubAdapter).subscriptions) == 1

    Process.exit(pid, :kill)
    assert_subscriptions_cleared(LocalStubAdapter)
  end

  defp assert_subscriptions_cleared(server) do
    result =
      Enum.reduce_while(1..50, :timeout, fn _, _ ->
        case :sys.get_state(server) do
          %{subscriptions: subs} when map_size(subs) == 0 ->
            {:halt, :ok}

          _ ->
            Process.sleep(10)
            {:cont, :timeout}
        end
      end)

    assert result == :ok,
           "expected #{inspect(server)} subscriptions to clear after subscriber DOWN"
  end
end
