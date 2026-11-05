defmodule AlchemyRateLimit do
  @moduledoc """
  ETS-backed fixed-window rate limiter (x0071).

  Apps start a named instance with their own table / telemetry prefix / config key.
  Hot-path `hit/4` reads ETS and Application env directly (no GenServer round-trip).
  """

  use GenServer

  require Logger

  @cleanup_interval_ms 5 * 60_000

  @type limit_config :: %{limit: pos_integer(), period_ms: pos_integer()}
  @type server :: atom()

  @doc """
  Starts a rate-limit GenServer.

  Required opts:

  - `:name` — registered process name (atom; also `hit/4` server key)
  - `:table` — ETS table atom
  - `:otp_app` — application for `Application.get_env/3`
  - `:config_key` — config key under `otp_app` (`:enabled`, `:limits`)
  - `:telemetry_prefix` — e.g. `[:auth, :rate_limit]` (event appends `:throttle`)

  Optional:

  - `:buckets` — keyword list `bucket => {action, axis}` for cleanup / telemetry labels
  - `:log_event` — string for structured log `event` field (default `"rate_limit.throttle"`)
  """
  def start_link(opts) do
    name = Keyword.fetch!(opts, :name)

    unless is_atom(name) do
      raise ArgumentError, "AlchemyRateLimit :name must be an atom, got: #{inspect(name)}"
    end

    GenServer.start_link(__MODULE__, opts, name: name)
  end

  @spec hit(server(), atom(), String.t(), limit_config()) :: :ok | {:error, :rate_limited}
  def hit(server, bucket, key, %{limit: limit, period_ms: period_ms})
      when is_atom(server) and is_atom(bucket) and is_binary(key) and is_integer(limit) and
             limit > 0 and is_integer(period_ms) and period_ms > 0 do
    cfg = instance_config!(server)

    if enabled?(cfg) do
      do_hit(cfg, bucket, key, limit, period_ms)
    else
      :ok
    end
  end

  @doc false
  def reset(server) when is_atom(server) do
    cfg = instance_config!(server)

    if table_exists?(cfg.table) do
      :ets.delete_all_objects(cfg.table)
    end

    :ok
  end

  @impl true
  def init(opts) do
    name = Keyword.fetch!(opts, :name)
    table = Keyword.fetch!(opts, :table)
    otp_app = Keyword.fetch!(opts, :otp_app)
    config_key = Keyword.fetch!(opts, :config_key)
    telemetry_prefix = Keyword.fetch!(opts, :telemetry_prefix)
    buckets = Keyword.get(opts, :buckets, [])
    log_event = Keyword.get(opts, :log_event, "rate_limit.throttle")

    :ets.new(table, [
      :named_table,
      :public,
      :set,
      write_concurrency: true,
      read_concurrency: true
    ])

    cfg = %{
      table: table,
      otp_app: otp_app,
      config_key: config_key,
      telemetry_prefix: telemetry_prefix,
      buckets: buckets,
      log_event: log_event
    }

    :persistent_term.put(persistent_key(name), cfg)

    schedule_cleanup()
    {:ok, Map.put(cfg, :name, name)}
  end

  @impl true
  def terminate(_reason, state) do
    :persistent_term.erase(persistent_key(state.name))
    :ok
  end

  @impl true
  def handle_info(:cleanup, state) do
    cleanup_stale_windows(state)
    schedule_cleanup()
    {:noreply, state}
  end

  defp instance_config!(server) do
    :persistent_term.get(persistent_key(server))
  rescue
    ArgumentError ->
      reraise ArgumentError,
              [message: "AlchemyRateLimit instance #{inspect(server)} is not started"],
              __STACKTRACE__
  end

  defp persistent_key(server), do: {__MODULE__, server}

  defp enabled?(cfg) do
    Application.get_env(cfg.otp_app, cfg.config_key, [])
    |> Keyword.get(:enabled, true)
  end

  defp do_hit(cfg, bucket, key, limit, period_ms) do
    window = div(System.system_time(:millisecond), period_ms)
    ets_key = {bucket, key, window}

    count = :ets.update_counter(cfg.table, ets_key, {2, 1}, {ets_key, 0})

    if count > limit do
      if count == limit + 1 do
        emit_throttle(cfg, bucket)
      end

      {:error, :rate_limited}
    else
      :ok
    end
  end

  defp emit_throttle(cfg, bucket) do
    {action, axis} = bucket_parts(cfg.buckets, bucket)

    :telemetry.execute(
      cfg.telemetry_prefix ++ [:throttle],
      %{count: 1},
      %{action: action, axis: axis, bucket: bucket}
    )

    Logger.warning(
      inspect(%{
        event: cfg.log_event,
        action: action,
        axis: axis,
        bucket: bucket
      })
    )
  end

  defp bucket_parts(buckets, bucket) do
    case Keyword.fetch(buckets, bucket) do
      {:ok, parts} -> parts
      :error -> {:unknown, bucket}
    end
  end

  defp table_exists?(table) do
    :ets.whereis(table) != :undefined
  end

  defp schedule_cleanup do
    Process.send_after(self(), :cleanup, @cleanup_interval_ms)
  end

  defp cleanup_stale_windows(state) do
    if table_exists?(state.table) do
      now = System.system_time(:millisecond)

      limits =
        Application.get_env(state.otp_app, state.config_key, []) |> Keyword.get(:limits, %{})

      for {bucket, {action, axis}} <- state.buckets do
        case get_in(limits, [action, axis, :period_ms]) do
          period_ms when is_integer(period_ms) and period_ms > 0 ->
            cutoff_window = div(now, period_ms) - 2

            :ets.select_delete(state.table, [
              {{{bucket, :_, :"$1"}, :_}, [{:<, :"$1", cutoff_window}], [true]}
            ])

          _ ->
            :ok
        end
      end
    end
  end
end
