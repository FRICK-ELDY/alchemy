defmodule World.Rooms.RoomKeyGrace do
  @moduledoc """
  In-memory previous Room key for host dual-open grace (x007e).

  Guarantees: after rotate, previous `(key, epoch)` is retained until
  `room_token_ttl_seconds` elapses (≈ max Token TTL). Not durable across
  world-server restart — post-restart only the current key is valid.

  **Consumers:** `RoomController.issue_token/6` reads this via `get/1` and,
  for **host** Tokens only, embeds `room_key_prev` + `room_key_prev_epoch`
  so a host that missed the live rotate (restart / late sync) can seed
  `SharedRoomAead` prev. Guests never receive prev.

  Non-guarantees: multi-node shared grace; client-side dual key buffer;
  Allocation JWT carriage of prev.
  """

  @table :world_room_key_grace

  @type prev :: %{
          prev_key: String.t(),
          prev_epoch: non_neg_integer(),
          expires_at: integer()
        }

  @spec ensure_table() :: :ok
  def ensure_table do
    case :ets.whereis(@table) do
      :undefined ->
        :ets.new(@table, [:named_table, :public, :set, read_concurrency: true])
        :ok

      _tid ->
        :ok
    end
  end

  @doc "Store previous key material after a successful rotate."
  @spec put(String.t(), String.t(), non_neg_integer()) :: :ok
  def put(room_id, prev_key_hex, prev_epoch)
      when is_binary(room_id) and is_binary(prev_key_hex) and is_integer(prev_epoch) and
             prev_epoch >= 0 do
    ensure_table()
    ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
    expires_at = System.system_time(:second) + ttl

    :ets.insert(
      @table,
      {room_id, %{prev_key: prev_key_hex, prev_epoch: prev_epoch, expires_at: expires_at}}
    )

    :ok
  end

  @doc "Return previous key if still within grace; otherwise `:miss`."
  @spec get(String.t()) :: {:ok, prev()} | :miss
  def get(room_id) when is_binary(room_id) do
    ensure_table()
    now = System.system_time(:second)

    case :ets.lookup(@table, room_id) do
      [{^room_id, %{expires_at: exp} = prev}] when is_integer(exp) and now <= exp ->
        {:ok, prev}

      [{^room_id, _}] ->
        :ets.delete(@table, room_id)
        :miss

      [] ->
        :miss
    end
  end

  def get(_), do: :miss

  @doc false
  @spec clear(String.t()) :: :ok
  def clear(room_id) when is_binary(room_id) do
    ensure_table()
    :ets.delete(@table, room_id)
    :ok
  end

  def clear(_), do: :ok
end
