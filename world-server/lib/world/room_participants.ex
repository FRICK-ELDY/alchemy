defmodule World.RoomParticipants do
  @moduledoc """
  In-memory Room Token issuance ledger for SpawnTicket `sub` checks (x00b5).

  Guarantees: `note/3` on successful Room Token issue; `participant?/2` is true when
  an unexpired ledger entry exists **or** `sub` is room `owner_sub` / decoded
  `editor_subs` / `active_host_sub` (same JSON decode as `Rooms.ensure_scene_writer/2`).
  Non-guarantees: persistence across world restart (re-issue Token or rely on room roles);
  presence / host SessionTable as authority.
  """

  alias World.Rooms
  alias World.Rooms.Room

  @table __MODULE__
  @prune_interval_ms 30_000
  @prune_meta_key {__MODULE__, :last_prune_ms}

  @doc false
  def ensure_table! do
    case :ets.whereis(@table) do
      :undefined ->
        :ets.new(@table, [
          :named_table,
          :public,
          :set,
          read_concurrency: true,
          write_concurrency: true
        ])

      _ ->
        @table
    end
  end

  @doc "Record that `sub` received a Room Token for `room_id` until unix `exp`."
  @spec note(String.t(), String.t(), integer()) :: :ok
  def note(room_id, sub, exp)
      when is_binary(room_id) and is_binary(sub) and is_integer(exp) do
    ensure_table!()
    maybe_prune_expired!()
    key = {room_id, sub}

    case :ets.lookup(@table, key) do
      [{^key, old_exp}] when is_integer(old_exp) and old_exp >= exp ->
        :ok

      _ ->
        :ets.insert(@table, {key, exp})
        :ok
    end
  end

  def note(_, _, _), do: :ok

  @doc "True when `sub` may author a SpawnTicket placement in `room_id`."
  @spec participant?(String.t(), String.t()) :: boolean()
  def participant?(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    ensure_table!()
    # Placement-only traffic (no Token re-issue) must still age out unused ledger rows.
    maybe_prune_expired!()
    ledger_member?(room_id, sub) or room_role_participant?(room_id, sub)
  end

  def participant?(_, _), do: false

  defp ledger_member?(room_id, sub) do
    now = System.system_time(:second)
    key = {room_id, sub}

    case :ets.lookup(@table, key) do
      [{^key, exp}] when is_integer(exp) and exp > now ->
        true

      [{^key, _expired}] ->
        :ets.delete(@table, key)
        false

      _ ->
        false
    end
  end

  defp maybe_prune_expired! do
    now_ms = System.monotonic_time(:millisecond)
    last = :persistent_term.get(@prune_meta_key, 0)

    if now_ms - last >= @prune_interval_ms do
      :persistent_term.put(@prune_meta_key, now_ms)
      now = System.system_time(:second)

      :ets.select_delete(@table, [
        {
          {:"$1", :"$2"},
          [{:"=<", :"$2", now}],
          [true]
        }
      ])

      :ok
    else
      :ok
    end
  end

  defp room_role_participant?(room_id, sub) do
    case Rooms.get(room_id) do
      {:ok, %Room{} = room} ->
        room.owner_sub == sub or
          sub in Rooms.editor_subs(room) or
          room.active_host_sub == sub

      {:error, _} ->
        false

      _ ->
        false
    end
  rescue
    # Sandbox / transient storage: treat as non-participant (ledger path still works).
    DBConnection.OwnershipError -> false
  end
end
