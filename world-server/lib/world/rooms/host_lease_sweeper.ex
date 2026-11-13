defmodule World.Rooms.HostLeaseSweeper do
  @moduledoc """
  Periodic host-lease expiry (x0080).

  Guarantees: active rooms whose `host_lease_expires_at` is past are moved to
  `dormant` (clears `active_host_sub`) so Public catalog does not keep ghost sessions.

  Non-guarantees: exact wall-clock alignment; multi-node coordination.

  Config `:world, :host_lease_sweep_ms` (config.exs default 15_000;
  `HOST_LEASE_SWEEP_MS` in runtime.exs). `0` or non-positive disables the ticker
  (ExUnit: tests call `Rooms.expire_due_host_leases/0` directly so Sandbox owners
  are not raced by background UPDATEs).
  """

  use GenServer

  require Logger

  @default_tick_ms 15_000

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl GenServer
  def init(_opts) do
    interval = tick_ms()
    if interval > 0, do: schedule_tick(interval)
    {:ok, %{interval: interval}}
  end

  @impl GenServer
  def handle_info(:tick, %{interval: interval} = state) when interval > 0 do
    {ok_n, err_n} = World.Rooms.expire_due_host_leases()

    if ok_n > 0 or err_n > 0 do
      Logger.info("host lease sweep: expired=#{ok_n} err=#{err_n}")
    end

    next = tick_ms()
    if next > 0, do: schedule_tick(next)
    {:noreply, %{state | interval: next}}
  end

  def handle_info(:tick, state) do
    {:noreply, state}
  end

  defp schedule_tick(interval) when is_integer(interval) and interval > 0 do
    Process.send_after(self(), :tick, interval)
  end

  defp tick_ms do
    case Application.get_env(:world, :host_lease_sweep_ms, @default_tick_ms) do
      n when is_integer(n) and n > 0 -> n
      _ -> 0
    end
  end
end
