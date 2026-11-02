defmodule World.Network.Zenoh.ZenohexAdapter do
  @moduledoc """
  Production Zenoh adapter backed by `zenohex` (Rustler NIF).

  Guarantees: sole module that calls `Zenohex.*`; `%Zenohex.Sample{}` never
  leaves this module (forwarder normalizes to
  `World.Network.Zenoh.Adapter.sample_message/0`); `:ring` channel config is
  assumed from app config; failed opens retry every 3s without crashing the
  supervisor; publishers are cached per `key_expr`; dead subscriber pids are
  cleaned via `Process.monitor`.
  Non-guarantees: mTLS client identity; ACL; topic schema; delivery when
  disconnected (`{:error, :not_connected}`); **session survival after a
  successful open** (no liveliness/`closed?` poll loop); **automatic
  re-subscribe** after reconnect — callers must treat disconnect as lost
  subscriptions; GenServer.call serialization (default 5s timeout) under NIF
  stall.
  """

  @behaviour World.Network.Zenoh.Adapter

  use GenServer

  require Logger

  alias World.Network.Zenoh.Adapter
  alias Zenohex.ConfigMap
  alias Zenohex.Nif.Logger, as: NifLogger
  alias Zenohex.Publisher
  alias Zenohex.Session
  alias Zenohex.Subscriber

  @reconnect_ms 3_000
  @call_timeout 5_000

  @type sub_entry :: %{
          nif_id: reference(),
          forwarder: pid(),
          subscriber_pid: pid(),
          monitor_ref: reference()
        }

  @type state :: %{
          session_id: reference() | nil,
          publishers: %{String.t() => reference()},
          subscriptions: %{reference() => sub_entry()},
          reconnect_ref: reference() | nil
        }

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc false
  @spec normalize_sample(Zenohex.Sample.t()) :: Adapter.sample_message()
  def normalize_sample(%Zenohex.Sample{key_expr: key_expr, payload: payload}) do
    Adapter.sample_message(key_expr, payload)
  end

  @impl World.Network.Zenoh.Adapter
  def publish(key_expr, payload)
      when is_binary(key_expr) and is_binary(payload) do
    GenServer.call(__MODULE__, {:publish, key_expr, payload}, @call_timeout)
  end

  @impl World.Network.Zenoh.Adapter
  def subscribe(key_expr, subscriber_pid)
      when is_binary(key_expr) and is_pid(subscriber_pid) do
    GenServer.call(__MODULE__, {:subscribe, key_expr, subscriber_pid}, @call_timeout)
  end

  @impl World.Network.Zenoh.Adapter
  def unsubscribe(sub_id) do
    GenServer.call(__MODULE__, {:unsubscribe, sub_id}, @call_timeout)
  end

  @impl GenServer
  def init(_opts) do
    _ = NifLogger.enable()
    _ = NifLogger.set_level(:warning)
    _ = NifLogger.set_target("zenohex_nif")

    state = %{
      session_id: nil,
      publishers: %{},
      subscriptions: %{},
      reconnect_ref: nil
    }

    {:ok, schedule_connect(state)}
  end

  @impl GenServer
  def handle_call({:publish, _key_expr, _payload}, _from, %{session_id: nil} = state) do
    {:reply, {:error, :not_connected}, state}
  end

  def handle_call({:publish, key_expr, payload}, _from, state) do
    case publisher_for(key_expr, state) do
      {:ok, publisher_id, state} ->
        case Publisher.put(publisher_id, payload) do
          :ok ->
            {:reply, :ok, state}

          {:error, reason} ->
            {:reply, {:error, reason}, maybe_recover_closed_session(state)}
        end

      {:error, reason, state} ->
        {:reply, {:error, reason}, maybe_recover_closed_session(state)}
    end
  end

  def handle_call({:subscribe, _key_expr, _pid}, _from, %{session_id: nil} = state) do
    {:reply, {:error, :not_connected}, state}
  end

  def handle_call({:subscribe, key_expr, subscriber_pid}, _from, state) do
    sub_id = make_ref()
    forwarder = spawn_forwarder(subscriber_pid)

    case Session.declare_subscriber(state.session_id, key_expr, forwarder) do
      {:ok, nif_id} ->
        entry = %{
          nif_id: nif_id,
          forwarder: forwarder,
          subscriber_pid: subscriber_pid,
          monitor_ref: Process.monitor(subscriber_pid)
        }

        subscriptions = Map.put(state.subscriptions, sub_id, entry)
        {:reply, {:ok, sub_id}, %{state | subscriptions: subscriptions}}

      {:error, reason} ->
        stop_forwarder(forwarder)
        {:reply, {:error, reason}, state}
    end
  end

  def handle_call({:unsubscribe, sub_id}, _from, state) do
    case Map.pop(state.subscriptions, sub_id) do
      {nil, _} ->
        {:reply, {:error, :not_found}, state}

      {entry, subscriptions} ->
        _ = drop_subscription_entry(entry)
        {:reply, :ok, %{state | subscriptions: subscriptions}}
    end
  end

  @impl GenServer
  def handle_info(:connect, state) do
    state = %{state | reconnect_ref: nil}

    case open_session() do
      {:ok, session_id} ->
        Logger.info("ZenohexAdapter: session open connect=#{connect_endpoint()}")
        # Non-guarantee: existing callers are NOT re-subscribed; drop local bookkeeping only.
        state = drop_all_subscriptions(state)
        {:noreply, %{state | session_id: session_id, publishers: %{}}}

      {:error, reason} ->
        Logger.warning(
          "ZenohexAdapter: session open failed: #{inspect(reason)}; retry in #{@reconnect_ms}ms"
        )

        {:noreply, schedule_reconnect(state)}
    end
  end

  def handle_info({:DOWN, monitor_ref, :process, _pid, _reason}, state) do
    {dropped, kept} =
      Enum.split_with(state.subscriptions, fn {_id, entry} ->
        entry.monitor_ref == monitor_ref
      end)

    Enum.each(dropped, fn {_id, entry} -> drop_subscription_entry(entry) end)
    {:noreply, %{state | subscriptions: Map.new(kept)}}
  end

  def handle_info(msg, state) do
    Logger.debug("ZenohexAdapter: unexpected message #{inspect(msg)}")
    {:noreply, state}
  end

  defp publisher_for(key_expr, state) do
    case Map.fetch(state.publishers, key_expr) do
      {:ok, publisher_id} ->
        {:ok, publisher_id, state}

      :error ->
        case Session.declare_publisher(state.session_id, key_expr) do
          {:ok, publisher_id} ->
            publishers = Map.put(state.publishers, key_expr, publisher_id)
            {:ok, publisher_id, %{state | publishers: publishers}}

          {:error, reason} ->
            {:error, reason, state}
        end
    end
  end

  defp open_session do
    endpoint = connect_endpoint()

    patch = %{
      mode: "client",
      connect: %{endpoints: [endpoint]}
    }

    with {:ok, config} <- ConfigMap.merge(ConfigMap.default(), patch) do
      Session.open(config)
    end
  end

  defp connect_endpoint do
    Application.get_env(:world, :zenoh_connect, "tcp/127.0.0.1:7447")
  end

  defp schedule_connect(state) do
    send(self(), :connect)
    state
  end

  defp schedule_reconnect(state) do
    cancel_reconnect(state)
    ref = Process.send_after(self(), :connect, @reconnect_ms)
    %{state | reconnect_ref: ref}
  end

  defp cancel_reconnect(%{reconnect_ref: nil}), do: :ok

  defp cancel_reconnect(%{reconnect_ref: ref}) do
    _ = Process.cancel_timer(ref)
    :ok
  end

  defp maybe_recover_closed_session(%{session_id: nil} = state), do: state

  defp maybe_recover_closed_session(%{session_id: session_id} = state) do
    if Session.closed?(session_id) do
      Logger.warning("ZenohexAdapter: session closed; scheduling reconnect")

      state
      |> drop_all_subscriptions()
      |> then(&%{&1 | session_id: nil, publishers: %{}})
      |> schedule_reconnect()
    else
      state
    end
  end

  defp drop_all_subscriptions(state) do
    Enum.each(state.subscriptions, fn {_id, entry} -> drop_subscription_entry(entry) end)
    %{state | subscriptions: %{}}
  end

  defp drop_subscription_entry(entry) do
    Process.demonitor(entry.monitor_ref, [:flush])
    stop_forwarder(entry.forwarder)
    _ = Subscriber.undeclare(entry.nif_id)
    :ok
  end

  defp spawn_forwarder(subscriber_pid) do
    spawn(fn -> forward_loop(subscriber_pid) end)
  end

  defp forward_loop(subscriber_pid) do
    receive do
      %Zenohex.Sample{} = sample ->
        send(subscriber_pid, normalize_sample(sample))
        forward_loop(subscriber_pid)

      :stop ->
        :ok
    end
  end

  defp stop_forwarder(forwarder) when is_pid(forwarder) do
    send(forwarder, :stop)
    :ok
  end
end
