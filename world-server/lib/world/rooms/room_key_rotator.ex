defmodule World.Rooms.RoomKeyRotator do
  @moduledoc """
  Periodic Room key rotation (x007e).

  Guarantees: on each tick, rooms with `room_key_rotated_at` older than
  `:room_key_rotate_hours` (env `ROOM_KEY_ROTATE_HOURS`, default 24) are rotated.
  Hours `0` disables the ticker without removing the child.

  Non-guarantees: exact wall-clock alignment; multi-node coordination.
  """

  use GenServer

  require Logger

  @default_hours 24
  @tick_ms :timer.minutes(15)

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @impl GenServer
  def init(_opts) do
    hours = rotate_hours()
    state = %{hours: hours}
    if hours > 0, do: schedule_tick()
    {:ok, state}
  end

  @impl GenServer
  def handle_info(:tick, %{hours: hours} = state) when hours > 0 do
    {ok_n, err_n} = World.Rooms.rotate_due_room_keys(hours)

    if ok_n > 0 or err_n > 0 do
      Logger.info("room key periodic rotate: ok=#{ok_n} err=#{err_n} hours=#{hours}")
    end

    schedule_tick()
    {:noreply, %{state | hours: rotate_hours()}}
  end

  def handle_info(:tick, state) do
    # Disabled: still wake occasionally so env flip to >0 takes effect without restart.
    schedule_tick()
    {:noreply, %{state | hours: rotate_hours()}}
  end

  defp schedule_tick do
    Process.send_after(self(), :tick, @tick_ms)
  end

  defp rotate_hours do
    case Application.get_env(:world, :room_key_rotate_hours, @default_hours) do
      h when is_integer(h) and h >= 0 -> h
      _ -> @default_hours
    end
  end
end
