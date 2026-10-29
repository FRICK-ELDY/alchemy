defmodule World.RateLimit do
  @moduledoc """
  ETS fixed-window rate limiter (x004c · room create).

  Guarantees: per-bucket key windows; disabled in test via config.
  Non-guarantees: multi-node shared quotas; sliding window.
  """

  use GenServer

  require Logger

  @table :world_rate_limit
  @cleanup_interval_ms 5 * 60_000

  # {config path under :limits, ETS bucket atom}
  @bucket_specs [
    {[:room_create, :owner], :room_create_owner}
  ]

  @type limit_config :: %{limit: pos_integer(), period_ms: pos_integer()}

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec hit(atom(), String.t(), limit_config()) :: :ok | {:error, :rate_limited}
  def hit(bucket, key, %{limit: limit, period_ms: period_ms})
      when is_atom(bucket) and is_binary(key) and is_integer(limit) and is_integer(period_ms) do
    if enabled?() do
      do_hit(bucket, key, limit, period_ms)
    else
      :ok
    end
  end

  @doc "Seconds until the current fixed window ends (min 1). For `Retry-After`."
  @spec retry_after_seconds(pos_integer()) :: pos_integer()
  def retry_after_seconds(period_ms)
      when is_integer(period_ms) and period_ms > 0 do
    now = System.system_time(:millisecond)
    remaining_ms = period_ms - rem(now, period_ms)

    remaining_ms
    |> div(1000)
    |> max(1)
  end

  @doc false
  def reset do
    if table_exists?() do
      :ets.delete_all_objects(@table)
    end

    :ok
  end

  @doc false
  def bucket_specs, do: @bucket_specs

  @impl true
  def init(_opts) do
    :ets.new(@table, [
      :named_table,
      :public,
      :set,
      write_concurrency: true,
      read_concurrency: true
    ])

    schedule_cleanup()
    {:ok, %{}}
  end

  @impl true
  def handle_info(:cleanup, state) do
    cleanup_stale_windows()
    schedule_cleanup()
    {:noreply, state}
  end

  defp do_hit(bucket, key, limit, period_ms) do
    window = div(System.system_time(:millisecond), period_ms)
    ets_key = {bucket, key, window}
    count = :ets.update_counter(@table, ets_key, {2, 1}, {ets_key, 0})

    if count > limit do
      if count == limit + 1 do
        Logger.warning(inspect(%{event: "world.rate_limit.throttle", bucket: bucket}))
      end

      {:error, :rate_limited}
    else
      :ok
    end
  end

  defp enabled? do
    Application.get_env(:world, __MODULE__, [])
    |> Keyword.get(:enabled, true)
  end

  defp table_exists? do
    :ets.whereis(@table) != :undefined
  end

  defp schedule_cleanup do
    Process.send_after(self(), :cleanup, @cleanup_interval_ms)
  end

  defp cleanup_stale_windows do
    if table_exists?() do
      now = System.system_time(:millisecond)
      limits = Application.get_env(:world, __MODULE__, []) |> Keyword.get(:limits, %{})
      Enum.each(@bucket_specs, &cleanup_bucket(&1, now, limits))
    end
  end

  defp cleanup_bucket({path, bucket}, now, limits) do
    case get_in(limits, path ++ [:period_ms]) do
      period_ms when is_integer(period_ms) and period_ms > 0 ->
        cutoff_window = div(now, period_ms) - 2

        :ets.select_delete(@table, [
          {{{bucket, :_, :"$1"}, :_}, [{:<, :"$1", cutoff_window}], [true]}
        ])

      _ ->
        :ok
    end
  end
end
