defmodule World.Rooms.HostLeaseSweeper do
  @moduledoc """
  Periodic host-lease expiry (x0080).

  Guarantees: active rooms whose `host_lease_expires_at` is past are moved to
  `dormant` (clears `active_host_sub`) so Public catalog does not keep ghost sessions.

  Non-guarantees: exact wall-clock alignment; multi-node coordination.
  """

  use GenServer

  require Logger

  @tick_ms :timer.seconds(15)

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl GenServer
  def init(_opts) do
    schedule_tick()
    {:ok, %{}}
  end

  @impl GenServer
  def handle_info(:tick, state) do
    {ok_n, err_n} = World.Rooms.expire_due_host_leases()

    if ok_n > 0 or err_n > 0 do
      Logger.info("host lease sweep: expired=#{ok_n} err=#{err_n}")
    end

    schedule_tick()
    {:noreply, state}
  end

  defp schedule_tick do
    Process.send_after(self(), :tick, @tick_ms)
  end
end
