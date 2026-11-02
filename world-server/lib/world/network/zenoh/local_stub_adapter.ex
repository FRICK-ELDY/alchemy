defmodule World.Network.Zenoh.LocalStubAdapter do
  @moduledoc """
  In-memory Zenoh adapter for `mix test` (no NIF session / zenohd).

  Guarantees: process-local publish fans out to exact-match subscribers as
  `World.Network.Zenoh.Adapter.sample_message/0`; dead subscriber pids are
  dropped via `Process.monitor`.
  Non-guarantees: key-expression wildcards; durability; multi-node fanout.
  """

  @behaviour World.Network.Zenoh.Adapter

  use GenServer

  alias World.Network.Zenoh.Adapter

  @type entry :: %{
          key_expr: String.t(),
          subscriber_pid: pid(),
          monitor_ref: reference()
        }

  @type state :: %{subscriptions: %{reference() => entry()}}

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl World.Network.Zenoh.Adapter
  def publish(key_expr, payload)
      when is_binary(key_expr) and is_binary(payload) do
    GenServer.call(__MODULE__, {:publish, key_expr, payload})
  end

  @impl World.Network.Zenoh.Adapter
  def subscribe(key_expr, subscriber_pid)
      when is_binary(key_expr) and is_pid(subscriber_pid) do
    GenServer.call(__MODULE__, {:subscribe, key_expr, subscriber_pid})
  end

  @impl World.Network.Zenoh.Adapter
  def unsubscribe(sub_id) do
    GenServer.call(__MODULE__, {:unsubscribe, sub_id})
  end

  @impl GenServer
  def init(_opts) do
    {:ok, %{subscriptions: %{}}}
  end

  @impl GenServer
  def handle_call({:publish, key_expr, payload}, _from, state) do
    Enum.each(state.subscriptions, fn {_id, entry} ->
      if entry.key_expr == key_expr do
        Adapter.deliver_sample(entry.subscriber_pid, key_expr, payload)
      end
    end)

    {:reply, :ok, state}
  end

  def handle_call({:subscribe, key_expr, subscriber_pid}, _from, state) do
    sub_id = make_ref()
    monitor_ref = Process.monitor(subscriber_pid)

    entry = %{
      key_expr: key_expr,
      subscriber_pid: subscriber_pid,
      monitor_ref: monitor_ref
    }

    subscriptions = Map.put(state.subscriptions, sub_id, entry)
    {:reply, {:ok, sub_id}, %{state | subscriptions: subscriptions}}
  end

  def handle_call({:unsubscribe, sub_id}, _from, state) do
    case Map.pop(state.subscriptions, sub_id) do
      {nil, _} ->
        {:reply, {:error, :not_found}, state}

      {entry, subscriptions} ->
        Process.demonitor(entry.monitor_ref, [:flush])
        {:reply, :ok, %{state | subscriptions: subscriptions}}
    end
  end

  @impl GenServer
  def handle_info({:DOWN, monitor_ref, :process, _pid, _reason}, state) do
    subscriptions =
      state.subscriptions
      |> Enum.reject(fn {_id, entry} -> entry.monitor_ref == monitor_ref end)
      |> Map.new()

    {:noreply, %{state | subscriptions: subscriptions}}
  end

  def handle_info(_msg, state), do: {:noreply, state}
end
