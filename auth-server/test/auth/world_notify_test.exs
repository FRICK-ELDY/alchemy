defmodule Auth.WorldNotifyTest do
  use Auth.DataCase, async: false

  alias Auth.WorldNotify
  alias Auth.WorldNotify.Outbox

  defmodule WorldStubPlug do
    @moduledoc false
    import Plug.Conn

    def init(opts), do: opts

    def call(conn, opts) do
      agent = Keyword.fetch!(opts, :agent)

      case Agent.get_and_update(agent, fn
             [:fail | rest] -> {:fail, rest}
             rest -> {:ok, rest}
           end) do
        :fail ->
          conn
          |> put_resp_content_type("application/json")
          |> send_resp(503, ~s({"error":"down"}))

        :ok ->
          conn
          |> put_resp_content_type("application/json")
          |> send_resp(200, ~s({"ok":true}))
      end
    end
  end

  setup do
    previous_mode = Application.get_env(:auth, :world_notify_mode)
    previous_stub = Application.get_env(:auth, :world_notify_stub)
    previous_url = Application.get_env(:auth, :world_internal_url)
    Application.put_env(:auth, :world_notify_mode, :stub)

    on_exit(fn ->
      restore(:world_notify_mode, previous_mode)
      restore(:world_notify_stub, previous_stub)
      restore(:world_internal_url, previous_url)
    end)

    :ok
  end

  defp restore(key, nil), do: Application.delete_env(:auth, key)
  defp restore(key, value), do: Application.put_env(:auth, key, value)

  test "stub receives relation_changed payload (x00d7)" do
    parent = self()
    Application.put_env(:auth, :world_notify_stub, parent)

    row =
      Outbox.insert!(
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        "unfriend",
        ["aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"]
      )

    assert :ok = WorldNotify.after_commit(row)

    assert_receive {:relation_changed, event_id, "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "unfriend", owners},
                   500

    assert event_id == row.id
    assert "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa" in owners
    assert Outbox.get(row.id).status == "delivered"
  end

  test "http_deliver fails while world down then succeeds (x00d7)" do
    {:ok, agent} = Agent.start_link(fn -> [:fail, :ok] end)

    {:ok, listen} =
      :gen_tcp.listen(0, [:binary, active: false, reuseaddr: true, ip: {127, 0, 0, 1}])

    {:ok, port} = :inet.port(listen)
    :ok = :gen_tcp.close(listen)

    {:ok, pid} =
      Bandit.start_link(
        plug: {WorldStubPlug, [agent: agent]},
        scheme: :http,
        port: port,
        ip: {127, 0, 0, 1},
        thousand_island_options: [num_acceptors: 1]
      )

    on_exit(fn ->
      Process.exit(pid, :normal)

      try do
        Agent.stop(agent)
      catch
        :exit, _ -> :ok
      end
    end)

    Application.put_env(:auth, :world_notify_mode, :http)
    Application.put_env(:auth, :world_internal_url, "http://127.0.0.1:#{port}")

    payload = %{
      event_id: Ecto.UUID.generate(),
      sub_a: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
      sub_b: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
      reason: "unfollow",
      affected_owner_subs: ["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"]
    }

    assert {:error, {:unexpected_status, 503, _}} = WorldNotify.http_deliver(payload)
    assert :ok = WorldNotify.http_deliver(payload)
  end

  test "outbox claim bumps attempts; mark_retry then mark_delivered (x00d7)" do
    row =
      Outbox.insert!(
        "cccccccc-cccc-cccc-cccc-cccccccccccc",
        "dddddddd-dddd-dddd-dddd-dddddddddddd",
        "unfollow",
        ["dddddddd-dddd-dddd-dddd-dddddddddddd"]
      )

    [claimed] = Outbox.claim_due()
    assert claimed.id == row.id
    assert claimed.attempts == 1

    retried = Outbox.mark_retry(claimed, :timeout, 50)
    assert retried.status == "pending"
    assert retried.attempts == 1
    assert retried.last_error =~ "timeout"

    delivered = Outbox.mark_delivered(retried)
    assert delivered.status == "delivered"
    assert delivered.delivered_at
  end

  test "dispatcher drain 503 then 2xx marks delivered (x00d7)" do
    {:ok, agent} = Agent.start_link(fn -> [:fail, :ok] end)

    {:ok, listen} =
      :gen_tcp.listen(0, [:binary, active: false, reuseaddr: true, ip: {127, 0, 0, 1}])

    {:ok, port} = :inet.port(listen)
    :ok = :gen_tcp.close(listen)

    {:ok, pid} =
      Bandit.start_link(
        plug: {WorldStubPlug, [agent: agent]},
        scheme: :http,
        port: port,
        ip: {127, 0, 0, 1},
        thousand_island_options: [num_acceptors: 1]
      )

    on_exit(fn ->
      Process.exit(pid, :normal)

      try do
        Agent.stop(agent)
      catch
        :exit, _ -> :ok
      end
    end)

    Application.put_env(:auth, :world_notify_mode, :http)
    Application.put_env(:auth, :world_internal_url, "http://127.0.0.1:#{port}")

    row =
      Outbox.insert!(
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        "unfollow",
        ["ffffffff-ffff-ffff-ffff-ffffffffffff"]
      )

    assert Auth.WorldNotify.Dispatcher.drain_now() == 0
    pending = Outbox.get(row.id)
    assert pending.status == "pending"
    assert pending.attempts >= 1

    # Backoff would defer reclaim; force due for the second drain.
    pending
    |> Ecto.Changeset.change(%{next_attempt_at: DateTime.utc_now()})
    |> Auth.Repo.update!()

    assert Auth.WorldNotify.Dispatcher.drain_now() == 1
    assert Outbox.get(row.id).status == "delivered"
  end

  test "purge_delivered removes old rows (x00d7)" do
    row =
      Outbox.insert!(
        "11111111-1111-1111-1111-111111111111",
        "22222222-2222-2222-2222-222222222222",
        "unfriend",
        ["11111111-1111-1111-1111-111111111111"]
      )

    delivered = Outbox.mark_delivered(row)
    old = DateTime.add(DateTime.utc_now(), -10 * 86_400, :second)

    delivered
    |> Ecto.Changeset.change(%{delivered_at: old})
    |> Auth.Repo.update!()

    assert Outbox.purge_delivered(7) >= 1
    assert Outbox.get(row.id) == nil
  end
end
