defmodule Auth.WorldNotify.Dispatcher do
  @moduledoc """
  Polls `relation_outbox` and delivers pending rows to world (x00d7).

  Guarantees: 2xx marks delivered; non-2xx / transport errors schedule backoff retry;
  due rows are claimed with `FOR UPDATE SKIP LOCKED`; delivered rows are purged by age.
  Non-guarantees: cross-node lease beyond Postgres row locks; at-most-once before world
  `relation_events` record (after record, world returns already_applied).
  """

  use GenServer

  require Logger

  alias Auth.WorldNotify
  alias Auth.WorldNotify.Outbox

  @default_interval_ms 1_000
  @base_backoff_ms 1_000
  @max_backoff_ms 60_000
  @purge_every_drains 60

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc "Wake the poller immediately (after social TX commit)."
  @spec kick() :: :ok
  def kick do
    case Process.whereis(__MODULE__) do
      nil -> :ok
      pid -> send(pid, :drain)
    end

    :ok
  end

  @doc "Synchronously drain due pending rows (tests / ops)."
  @spec drain_now() :: non_neg_integer()
  def drain_now do
    case Process.whereis(__MODULE__) do
      nil -> do_drain()
      _pid -> GenServer.call(__MODULE__, :drain)
    end
  end

  @impl true
  def init(opts) do
    interval = Keyword.get(opts, :interval_ms, @default_interval_ms)
    schedule(interval)
    {:ok, %{interval_ms: interval, drains: 0}}
  end

  @impl true
  def handle_call(:drain, _from, state) do
    count = do_drain()
    {:reply, count, maybe_purge(%{state | drains: state.drains + 1})}
  end

  @impl true
  def handle_info(:drain, state) do
    _ = do_drain()
    state = maybe_purge(%{state | drains: state.drains + 1})
    schedule(state.interval_ms)
    {:noreply, state}
  end

  defp schedule(interval_ms) do
    Process.send_after(self(), :drain, interval_ms)
  end

  defp maybe_purge(%{drains: drains} = state) when rem(drains, @purge_every_drains) == 0 do
    purged = Outbox.purge_delivered()

    if purged > 0 do
      Logger.info("WorldNotify.Dispatcher purged delivered_outbox=#{purged}")
    end

    state
  end

  defp maybe_purge(state), do: state

  defp do_drain do
    Outbox.claim_due()
    |> Enum.reduce(0, fn row, acc ->
      case deliver_one(row) do
        :ok -> acc + 1
        :error -> acc
      end
    end)
  end

  defp deliver_one(%Outbox{} = row) do
    case WorldNotify.http_deliver(Outbox.payload(row)) do
      :ok ->
        _ = Outbox.mark_delivered(row)
        :ok

      {:error, reason} ->
        backoff = backoff_ms(row.attempts)
        _ = Outbox.mark_retry(row, reason, backoff)

        Logger.warning(
          "WorldNotify.Dispatcher retry event_id=#{row.id} reason=#{inspect(reason)}"
        )

        :error
    end
  end

  defp backoff_ms(attempts) when is_integer(attempts) and attempts >= 0 do
    exp = min(attempts, 6)
    min(@base_backoff_ms * Integer.pow(2, exp), @max_backoff_ms)
  end
end
