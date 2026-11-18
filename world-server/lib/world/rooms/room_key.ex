defmodule World.Rooms.RoomKey do
  @moduledoc """
  Room key rotation (x007e / x00a0 / **x00d7**; x00d6 split from `World.Rooms`).

  Guarantees: rotate / visibility-narrow / periodic due rotate bump `room_key_epoch` via epoch CAS
  (lost races retry) and park the previous key in `RoomKeyGrace`. Relation revoke rotates
  `needs_social?` rooms owned by `affected_owner_subs` (2hop upper bound from auth) with
  optional `event_id` idempotency via `World.RelationEvents`.

  Non-guarantees: owner/auth policy beyond `owner_matches/2` on `rotate_room_key/2`; visibility
  parsing (`World.Rooms.set_visibility/3` orchestrates and calls `narrow_visibility_with_rotate/2`).

  Callers should use the `World.Rooms` facade; this module is the implementation unit.
  """

  require Logger

  import Ecto.Query, only: [from: 2]

  import World.Rooms.Internal,
    only: [get_room: 1, owner_matches: 2, set_visibility_only: 2, storage_error: 2]

  alias World.RelationEvents
  alias World.Rooms.Room
  alias World.Rooms.RoomKeyGrace
  alias World.Visibility

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  # Same budget as host_generation CAS (x003a) — concurrent rotate / narrow must not
  # overwrite an intermediate epoch without parking its key in grace.
  @rotate_cas_attempts 8

  @doc """
  Owner-driven Room key revoke (x007e / x00a0). Same effect as visibility narrow without
  changing visibility. HTTP: `POST …/room_key/rotate`.
  """
  @spec rotate_room_key(room_id(), owner_sub()) ::
          {:ok, non_neg_integer() | :noop}
          | {:error, :invalid_room_id | :not_found | :forbidden_host | storage_error()}
  def rotate_room_key(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    with {:ok, %Room{} = room} <- get_room(room_id),
         :ok <- owner_matches(room, sub) do
      do_rotate_room_key(room)
    end
  end

  def rotate_room_key(_, _), do: {:error, :invalid_room_id}

  @doc """
  x00a0 / x00d7: after auth friendship/follow revoke, rotate social-visibility rooms owned by
  `affected_owner_subs` (defaults to either party). Optional `event_id` is recorded **after**
  a successful rotate batch (so 503 / partial failure stays retriable); already-recorded ids
  return `{:ok, :already_applied}`. Idempotent “epoch が二重に進まない” holds **only after
  record succeeds**; check-then-act races before record may over-rotate (safe). Mid-batch
  storage failure leaves the event unrecorded — auth outbox retry closes the rest.
  """
  @spec rotate_for_relation_change(owner_sub(), owner_sub()) ::
          {:ok,
           [%{room_id: room_id(), room_key_epoch: non_neg_integer() | :noop}] | :already_applied}
          | {:error, storage_error()}
  def rotate_for_relation_change(sub_a, sub_b), do: rotate_for_relation_change(sub_a, sub_b, [])

  @spec rotate_for_relation_change(owner_sub(), owner_sub(), keyword()) ::
          {:ok,
           [%{room_id: room_id(), room_key_epoch: non_neg_integer() | :noop}] | :already_applied}
          | {:error, storage_error()}
  def rotate_for_relation_change(sub_a, sub_b, opts)
      when is_binary(sub_a) and sub_a != "" and is_binary(sub_b) and sub_b != "" and is_list(opts) do
    owners = relation_affected_owners(opts, sub_a, sub_b)
    event_id = Keyword.get(opts, :event_id)
    rotate_relation_if_new(event_id, owners)
  end

  def rotate_for_relation_change(_, _, _), do: {:ok, []}

  defp relation_affected_owners(opts, sub_a, sub_b) do
    case Keyword.get(opts, :affected_owner_subs) do
      list when is_list(list) and list != [] ->
        list
        |> Enum.filter(&(is_binary(&1) and &1 != ""))
        |> Enum.uniq()

      _ ->
        [sub_a, sub_b]
    end
  end

  defp rotate_relation_if_new(event_id, owners) do
    case RelationEvents.applied?(event_id) do
      true -> {:ok, :already_applied}
      {:error, :storage_error} -> {:error, :storage_error}
      false -> rotate_and_record(event_id, owners)
    end
  end

  defp rotate_and_record(event_id, owners) do
    case rotate_social_rooms_for_owners(owners) do
      {:ok, rotated} -> record_relation_event(event_id, rotated)
      {:error, _} = err -> err
    end
  end

  defp record_relation_event(event_id, rotated) do
    case RelationEvents.record(event_id) do
      :ok ->
        {:ok, rotated}

      :already_applied ->
        # Concurrent deliver finished recording; rotate work already done this call.
        {:ok, rotated}

      {:error, :storage_error} ->
        # Leave unrecorded so auth retry can close; may over-rotate.
        {:error, :storage_error}
    end
  end

  defp rotate_social_rooms_for_owners([]), do: {:ok, []}

  defp rotate_social_rooms_for_owners(owners) when is_list(owners) do
    query =
      from r in Room,
        where: r.owner_sub in ^owners

    try do
      rooms = World.Repo.all(query)

      rooms
      |> Enum.filter(&Visibility.needs_social?(&1.visibility))
      |> Enum.reduce_while([], &accumulate_relation_rotate/2)
      |> finalize_relation_rotate()
    rescue
      error -> storage_error(error, :rotate_for_relation_change)
    end
  end

  defp accumulate_relation_rotate(room, acc) do
    case maybe_relation_rotate_test_inject(acc) do
      {:halt, _} = halt -> halt
      :cont -> do_accumulate_relation_rotate(room, acc)
    end
  end

  # Compiled only under MIX_ENV=test — absent from prod/dev beams (no runtime get_env).
  if Mix.env() == :test do
    defp maybe_relation_rotate_test_inject(acc) do
      case Application.get_env(:world, :relation_rotate_fail_after) do
        n when is_integer(n) and n >= 0 and length(acc) >= n ->
          {:halt, {:error, :storage_error}}

        _ ->
          :cont
      end
    end
  else
    defp maybe_relation_rotate_test_inject(_acc), do: :cont
  end

  defp do_accumulate_relation_rotate(room, acc) do
    case do_rotate_room_key(room) do
      {:ok, epoch} ->
        {:cont, [%{room_id: room.room_id, room_key_epoch: epoch} | acc]}

      {:error, :storage_error} ->
        # Do not record event_id (caller); auth retry re-runs the batch (over-rotate OK).
        {:halt, {:error, :storage_error}}

      {:error, _} ->
        {:cont, acc}
    end
  end

  defp finalize_relation_rotate({:error, _} = err), do: err
  defp finalize_relation_rotate(list) when is_list(list), do: {:ok, Enum.reverse(list)}

  @doc """
  Visibility narrow + room key rotate in one epoch CAS (x007e). `vis` is an already-parsed,
  narrower-than-current level; the caller (`World.Rooms.set_visibility/3`) owns parsing and
  owner authorization.
  """
  @spec narrow_visibility_with_rotate(Room.t(), String.t()) ::
          {:ok, String.t()} | {:error, storage_error() | :not_found | :invalid_room_id}
  def narrow_visibility_with_rotate(%Room{} = room, vis) when is_binary(vis) do
    cas_visibility_narrow_rotate(room, vis, @rotate_cas_attempts)
  end

  defp cas_visibility_narrow_rotate(_room, _vis, 0) do
    Logger.error("World.Rooms.set_visibility narrow+rotate CAS exhausted")
    {:error, :storage_error}
  end

  defp cas_visibility_narrow_rotate(%Room{} = room, vis, attempts_left) do
    case try_narrow_rotate_once(room, vis) do
      {:ok, _} = ok -> ok
      :conflict -> retry_narrow_after_conflict(room.room_id, vis, attempts_left - 1)
      {:error, error} -> storage_error(error, :set_visibility)
    end
  end

  defp try_narrow_rotate_once(%Room{} = room, vis) do
    expected_epoch = room.room_key_epoch
    new_key = generate_room_key_hex()
    new_epoch = expected_epoch + 1
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    prev_key = room.room_key
    prev_epoch = room.room_key_epoch

    sql = """
    UPDATE rooms
    SET visibility = ?, room_key = ?, room_key_epoch = ?, room_key_rotated_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND room_key_epoch = ?
    """

    case World.Repo.query(sql, [vis, new_key, new_epoch, now, room.room_id, expected_epoch]) do
      {:ok, %{num_rows: 1}} ->
        RoomKeyGrace.put(room.room_id, prev_key, prev_epoch)
        {:ok, vis}

      {:ok, %{num_rows: 0}} ->
        :conflict

      {:error, error} ->
        {:error, error}
    end
  end

  defp retry_narrow_after_conflict(room_id, vis, attempts_left) do
    case get_room(room_id) do
      {:ok, %Room{} = fresh} -> continue_narrow_after_conflict(fresh, vis, attempts_left)
      other -> other
    end
  end

  defp continue_narrow_after_conflict(%Room{} = fresh, vis, attempts_left) do
    cond do
      fresh.visibility == vis ->
        {:ok, vis}

      Visibility.narrower?(vis, fresh.visibility) ->
        cas_visibility_narrow_rotate(fresh, vis, attempts_left)

      true ->
        set_visibility_only(fresh, vis)
    end
  end

  # Owner / narrow: always retry on CAS miss. Periodic due: pass `due_cutoff:` so a
  # concurrent rotate that already refreshed `room_key_rotated_at` past the cutoff
  # does not bump another epoch (extra KeyStale).
  defp do_rotate_room_key(%Room{} = room, opts \\ []) do
    due_cutoff = Keyword.get(opts, :due_cutoff)
    do_rotate_room_key_cas(room, @rotate_cas_attempts, due_cutoff)
  end

  defp do_rotate_room_key_cas(_room, 0, _due_cutoff) do
    Logger.error("World.Rooms.rotate_room_key CAS exhausted")
    {:error, :storage_error}
  end

  defp do_rotate_room_key_cas(%Room{} = room, attempts_left, due_cutoff) do
    case try_rotate_once(room) do
      {:ok, _} = ok -> ok
      :conflict -> retry_rotate_after_conflict(room.room_id, attempts_left - 1, due_cutoff)
      {:error, error} -> storage_error(error, :rotate_room_key)
    end
  end

  defp try_rotate_once(%Room{} = room) do
    expected_epoch = room.room_key_epoch
    new_key = generate_room_key_hex()
    new_epoch = expected_epoch + 1
    now = DateTime.utc_now() |> DateTime.truncate(:second)
    prev_key = room.room_key
    prev_epoch = room.room_key_epoch

    sql = """
    UPDATE rooms
    SET room_key = ?, room_key_epoch = ?, room_key_rotated_at = ?,
        updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ?
      AND room_key_epoch = ?
    """

    case World.Repo.query(sql, [new_key, new_epoch, now, room.room_id, expected_epoch]) do
      {:ok, %{num_rows: 1}} ->
        RoomKeyGrace.put(room.room_id, prev_key, prev_epoch)
        {:ok, new_epoch}

      {:ok, %{num_rows: 0}} ->
        :conflict

      {:error, error} ->
        {:error, error}
    end
  end

  defp retry_rotate_after_conflict(room_id, attempts_left, due_cutoff) do
    case get_room(room_id) do
      {:ok, %Room{} = fresh} -> maybe_retry_rotate_cas(fresh, attempts_left, due_cutoff)
      other -> other
    end
  end

  defp maybe_retry_rotate_cas(%Room{} = fresh, attempts_left, due_cutoff) do
    if room_key_still_due_for_cas?(fresh, due_cutoff) do
      do_rotate_room_key_cas(fresh, attempts_left, due_cutoff)
    else
      # Concurrent rotate already satisfied the periodic window.
      {:ok, :noop}
    end
  end

  # `due_cutoff == nil` → owner / narrow revoke: always retry.
  defp room_key_still_due_for_cas?(_room, nil), do: true

  defp room_key_still_due_for_cas?(%Room{room_key_rotated_at: %DateTime{} = at}, cutoff) do
    DateTime.compare(at, cutoff) != :gt
  end

  defp room_key_still_due_for_cas?(%Room{room_key_rotated_at: nil}, _cutoff), do: false

  @doc """
  Rotate every room whose `room_key_rotated_at` is older than `hours` (x007e periodic).

  Returns `{rotated_count, errors}`. Hours `<= 0` skips (disabled).

  Null `room_key_rotated_at` is stamped to now **without** rotating (deploy baseline —
  do not treat `inserted_at` as due, which would KeyStale every pre-migration room at once).
  """
  @spec rotate_due_room_keys(pos_integer() | non_neg_integer()) ::
          {non_neg_integer(), non_neg_integer()}
  def rotate_due_room_keys(hours) when is_integer(hours) and hours <= 0, do: {0, 0}

  def rotate_due_room_keys(hours) when is_integer(hours) and hours > 0 do
    cutoff = DateTime.add(DateTime.utc_now(), -hours * 3600, :second)
    _ = stamp_null_room_key_rotated_at()

    case due_rooms_for_key_rotate(cutoff) do
      {:ok, rooms} ->
        Enum.reduce(rooms, {0, 0}, &accumulate_due_rotate(&1, &2, cutoff))

      {:error, _} ->
        {0, 1}
    end
  end

  defp stamp_null_room_key_rotated_at do
    now = DateTime.utc_now() |> DateTime.truncate(:second)

    case World.Repo.query(
           """
           UPDATE rooms
           SET room_key_rotated_at = ?, updated_at = CURRENT_TIMESTAMP
           WHERE room_key_rotated_at IS NULL
           """,
           [now]
         ) do
      {:ok, _} ->
        :ok

      {:error, error} ->
        _ = storage_error(error, :stamp_room_key_rotated_at)
        :error
    end
  end

  defp due_rooms_for_key_rotate(cutoff) do
    sql = """
    SELECT room_id FROM rooms
    WHERE room_key_rotated_at IS NOT NULL
      AND room_key_rotated_at <= ?
    """

    case World.Repo.query(sql, [cutoff]) do
      {:ok, %{rows: rows}} ->
        {:ok, load_rooms_by_id_rows(rows)}

      {:error, error} ->
        storage_error(error, :rotate_due_room_keys)
    end
  end

  defp load_rooms_by_id_rows(rows) do
    Enum.flat_map(rows, &load_room_row/1)
  end

  defp load_room_row([room_id]) do
    case get_room(room_id) do
      {:ok, %Room{} = room} -> [room]
      _ -> []
    end
  end

  defp accumulate_due_rotate(room, {ok_n, err_n}, cutoff) do
    case do_rotate_room_key(room, due_cutoff: cutoff) do
      {:ok, :noop} -> {ok_n, err_n}
      {:ok, _epoch} -> {ok_n + 1, err_n}
      {:error, _} -> {ok_n, err_n + 1}
    end
  end

  # x0069: 32 random bytes → 64 hex chars (Token JSON uses URL-safe base64 of raw bytes).
  def generate_room_key_hex do
    Base.encode16(:crypto.strong_rand_bytes(32), case: :lower)
  end

  @doc """
  Encode stored hex `room_key` to URL-safe base64 (no padding) for Token JSON.
  """
  def room_key_for_token(hex) when is_binary(hex) do
    case Base.decode16(hex, case: :lower) do
      {:ok, <<_::binary-size(32)>> = raw} ->
        {:ok, Base.url_encode64(raw, padding: false)}

      {:ok, _} ->
        {:error, :invalid_room_key}

      :error ->
        {:error, :invalid_room_key}
    end
  end
end
