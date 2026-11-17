defmodule World.Rooms.Internal do
  @moduledoc false

  require Logger

  alias World.Rooms.Room

  @type room_id :: String.t()
  @type storage_error :: :storage_error

  @revision_stride 1_000_000

  @doc false
  @spec revision_stride() :: pos_integer()
  def revision_stride, do: @revision_stride

  @doc false
  @spec get_room(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def get_room(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{} = room} -> {:ok, room}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :get)
      end
    end
  end

  def get_room(_), do: {:error, :invalid_room_id}

  @doc false
  def fetch_room(room_id, where \\ :fetch_room) do
    case Room.get_by_room_id(room_id) do
      {:ok, %Room{} = room} -> {:ok, room}
      {:ok, nil} -> {:error, :not_found}
      {:error, error} -> map_read_error(error, where)
    end
  end

  def empty_overlay, do: %{"revision" => 0, "entries" => []}

  # Bump overlay revision and drop entries so packed GenRev never regresses (x009e).
  # Call sites MUST also AND `overlay_clear_headroom_sql/0` so stride overflow fail-closes
  # (UPDATE num_rows=0) instead of writing revision >= 1_000_000.
  def cleared_overlay_sql do
    """
    json_object(
      'revision', COALESCE(CAST(json_extract(session_overlay, '$.revision') AS INTEGER), 0) + 1,
      'entries', json_array()
    )
    """
  end

  def overlay_clear_headroom_sql do
    "COALESCE(CAST(json_extract(session_overlay, '$.revision') AS INTEGER), 0) + 1 < #{@revision_stride}"
  end

  # Mirror SQL headroom predicate (`CAST(... AS INTEGER)` + `COALESCE(..., 0)`).
  # Do not use `require_overlay/1` here — corrupt revision must not force
  # `:invalid_revision` when the real miss was a state/CAS race.
  def overlay_lacks_clear_headroom?(%Room{} = room) do
    overlay_sql_style_revision(room.session_overlay) + 1 >= @revision_stride
  end

  defp overlay_sql_style_revision(overlay) when is_map(overlay) do
    case Map.get(overlay, "revision") || Map.get(overlay, :revision) do
      rev when is_integer(rev) ->
        rev

      rev when is_binary(rev) ->
        case Integer.parse(String.trim(rev)) do
          {n, ""} -> n
          _ -> 0
        end

      _ ->
        0
    end
  end

  defp overlay_sql_style_revision(bin) when is_binary(bin) do
    case Jason.decode(bin) do
      {:ok, map} when is_map(map) -> overlay_sql_style_revision(map)
      _ -> 0
    end
  end

  defp overlay_sql_style_revision(_), do: 0

  # Clear CAS missed: stride ceiling → :invalid_revision; otherwise CAS/state race → :conflict.
  def miss_after_overlay_clear(room_id) do
    case get_room(room_id) do
      {:ok, room} ->
        if overlay_lacks_clear_headroom?(room) do
          {:error, :invalid_revision}
        else
          {:error, :conflict}
        end

      {:error, _} = err ->
        err
    end
  end

  def set_visibility_only(%Room{} = room, vis) do
    case Room.set_visibility(room, %{visibility: vis}) do
      {:ok, %Room{visibility: updated_vis}} -> {:ok, updated_vis}
      {:error, error} -> storage_error(error, :set_visibility)
    end
  end

  def lease_valid?(%DateTime{} = expires_at) do
    DateTime.compare(expires_at, DateTime.utc_now()) == :gt
  end

  def lease_valid?(%NaiveDateTime{} = expires_at) do
    expires_at
    |> DateTime.from_naive!("Etc/UTC")
    |> lease_valid?()
  end

  # Raw SQL catalog rows may surface ISO strings from SQLite.
  def lease_valid?(expires_at) when is_binary(expires_at) do
    case DateTime.from_iso8601(expires_at) do
      {:ok, dt, _} ->
        lease_valid?(dt)

      _ ->
        case NaiveDateTime.from_iso8601(expires_at) do
          {:ok, ndt} -> lease_valid?(ndt)
          _ -> false
        end
    end
  end

  def lease_valid?(_), do: false

  def owner_matches(%Room{owner_sub: owner}, sub) when owner == sub, do: :ok
  def owner_matches(%Room{}, _), do: {:error, :forbidden_host}

  # x009d: `editor_subs` column is JSON text `["sub", …]`.
  def decode_editor_subs(nil), do: []

  def decode_editor_subs(bin) when is_binary(bin) do
    case Jason.decode(bin) do
      {:ok, list} when is_list(list) ->
        Enum.filter(list, &(is_binary(&1) and &1 != ""))

      _ ->
        []
    end
  end

  def decode_editor_subs(_), do: []

  @editor_subs_max 64

  def encode_editor_subs(editors) when is_list(editors) do
    unique = Enum.uniq(editors)

    cond do
      length(unique) > @editor_subs_max ->
        {:error, :invalid_editors}

      Enum.all?(unique, &(is_binary(&1) and &1 != "")) ->
        {:ok, Jason.encode!(unique)}

      true ->
        {:error, :invalid_editors}
    end
  end

  def encode_eligible_subs(subs) when is_list(subs) do
    case encode_editor_subs(subs) do
      {:ok, _} = ok -> ok
      {:error, :invalid_editors} -> {:error, :invalid_eligible}
    end
  end

  def map_create_error(error) do
    if unique_violation?(error) do
      {:error, :already_exists}
    else
      storage_error(error, :create)
    end
  end

  def map_read_error(error, where) do
    if not_found_error?(error) do
      {:error, :not_found}
    else
      storage_error(error, where)
    end
  end

  def storage_error(error, where) do
    Logger.error("World.Rooms.#{where} storage failure: #{Exception.message(error)}")
    {:error, :storage_error}
  end

  defp not_found_error?(error) do
    error
    |> ash_error_leaves()
    |> Enum.any?(fn
      %Ash.Error.Query.NotFound{} -> true
      _ -> false
    end) or Exception.message(error) =~ "record not found"
  end

  def unique_violation?(error) do
    error
    |> ash_error_leaves()
    |> Enum.any?(&unique_leaf?/1)
  end

  defp ash_error_leaves(%{errors: errors}) when is_list(errors) do
    Enum.flat_map(errors, &ash_error_leaves/1)
  end

  defp ash_error_leaves(error), do: [error]

  defp unique_leaf?(%Ash.Error.Changes.InvalidChanges{fields: fields, message: message})
       when is_binary(message) do
    fields = List.wrap(fields)
    room_id_field?(fields) and taken_message?(message)
  end

  defp unique_leaf?(%Ash.Error.Changes.InvalidAttribute{field: :room_id, message: message})
       when is_binary(message) do
    taken_message?(message)
  end

  defp unique_leaf?(%{constraint: :unique_constraint, constraint_name: name})
       when is_binary(name) do
    room_id_constraint_name?(name)
  end

  defp unique_leaf?(%{constraint: :unique_constraint, fields: fields}) do
    room_id_field?(List.wrap(fields))
  end

  defp unique_leaf?(%{constraint_name: name}) when is_binary(name) do
    room_id_constraint_name?(name)
  end

  # SQLite primary-key conflict surfaces as a data-layer / unknown error message.
  defp unique_leaf?(error) do
    message = Exception.message(error)

    (String.contains?(message, "UNIQUE constraint failed") or
       String.contains?(message, "unique constraint")) and
      (String.contains?(message, "room_id") or String.contains?(message, "rooms"))
  end

  defp room_id_field?(fields), do: :room_id in fields

  defp room_id_constraint_name?(name) when is_binary(name) do
    String.contains?(name, "rooms") and
      (String.contains?(name, "room_id") or String.contains?(name, "unique") or
         String.contains?(name, "pkey"))
  end

  defp taken_message?(message) when is_binary(message) do
    String.contains?(message, "already been taken")
  end

  def validate_room_id(room_id), do: World.RoomId.validate(room_id)
end
