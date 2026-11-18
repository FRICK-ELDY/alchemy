defmodule Assets.Vcs.PushGate do
  @moduledoc """
  Per-`sub` concurrency gate for Spatial VCS push (x00d9).

  Guarantees: at most one holder per `sub` on this node for the whole HTTP push
  critical section (**before body read through** `Assets.Vcs.push/3`); holder
  crash releases via process monitor. Callers that get `{:error, :busy}` must
  not read the request body. Non-guarantees: cross-node fairness; blake3 Port
  pool fairness.
  """

  use GenServer

  @type sub :: String.t()

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, %{}, Keyword.put_new(opts, :name, __MODULE__))
  end

  @doc """
  Acquire the push slot for `sub` on behalf of the calling process.

  Returns `:ok` or `{:error, :busy}` when another process already holds it.
  """
  @spec checkout(sub()) :: :ok | {:error, :busy}
  def checkout(sub) when is_binary(sub) and sub != "" do
    GenServer.call(__MODULE__, {:checkout, sub, self()})
  end

  @doc "Release the push slot for `sub` if held by the calling process."
  @spec checkin(sub()) :: :ok
  def checkin(sub) when is_binary(sub) and sub != "" do
    GenServer.call(__MODULE__, {:checkin, sub, self()})
  end

  @impl true
  def init(_opts) do
    {:ok, %{holders: %{}}}
  end

  @impl true
  def handle_call({:checkout, sub, pid}, _from, state) do
    case Map.get(state.holders, sub) do
      nil ->
        ref = Process.monitor(pid)
        {:reply, :ok, put_in(state.holders[sub], {pid, ref})}

      {^pid, _ref} ->
        {:reply, :ok, state}

      {_other, _ref} ->
        {:reply, {:error, :busy}, state}
    end
  end

  def handle_call({:checkin, sub, pid}, _from, state) do
    case Map.get(state.holders, sub) do
      {^pid, ref} ->
        Process.demonitor(ref, [:flush])
        {:reply, :ok, %{state | holders: Map.delete(state.holders, sub)}}

      _ ->
        {:reply, :ok, state}
    end
  end

  @impl true
  def handle_info({:DOWN, ref, :process, pid, _reason}, state) do
    holders =
      state.holders
      |> Enum.reject(fn {_sub, {holder_pid, holder_ref}} ->
        holder_pid == pid and holder_ref == ref
      end)
      |> Map.new()

    {:noreply, %{state | holders: holders}}
  end

  def handle_info(_msg, state), do: {:noreply, state}
end
