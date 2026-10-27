defmodule World.Rooms do
  @moduledoc """
  Room ledger facade (x0028).

  Guarantees: create / owner / scene / visibility backed by Ash + SQLite (survives restart).
  Non-guarantees: UGC scene edits; multi-node replication; Postgres (swap path documented);
  Room Token revocation on visibility / friendship change (TTL-only; see RoomAccess).

  x0021: `owner_sub` set only via `create/3` (`{:ok, room_id, visibility}`).
  Token issuance never creates a room.
  x0027: DemoScene assigned at create and returned by `scene/1`.



  Authorization: use `ensure_owner/2` (and `require_existing/1` for existence-only).
  Do not infer auth from boolean helpers — those APIs are intentionally absent so
  storage failures cannot collapse to `false` / `nil`.

  Errors: `:invalid_room_id` / `:already_exists` / `:not_found` are client-facing.
  `:storage_error` is infrastructure (SQLite busy, disk, Repo) → HTTP 5xx.
  """

  require Logger

  alias World.DemoScene
  alias World.Rooms.Room

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type storage_error :: :storage_error

  @spec create(room_id() | nil, owner_sub(), String.t()) ::
          {:ok, room_id(), String.t()}
          | {:error,
             :invalid_room_id
             | :already_exists
             | :missing_owner
             | :invalid_visibility
             | storage_error()}
  def create(room_id, owner_sub, visibility \\ "public")

  def create(nil, owner_sub, visibility), do: create(random_room_id(), owner_sub, visibility)

  def create(_room_id, owner_sub, _visibility)
      when not is_binary(owner_sub) or owner_sub == "" do
    {:error, :missing_owner}
  end

  def create(room_id, owner_sub, visibility)
      when is_binary(room_id) and is_binary(owner_sub) and is_binary(visibility) do
    with :ok <- validate_room_id(room_id),
         {:ok, vis} <- World.Visibility.parse(visibility),
         {:ok, %Room{room_id: id, visibility: stored}} <- insert_room(room_id, owner_sub, vis) do
      {:ok, id, stored}
    else
      {:error, :invalid_room_id} = err -> err
      {:error, :invalid_visibility} = err -> err
      {:error, :already_exists} = err -> err
      {:error, :storage_error} = err -> err
      {:error, error} -> map_create_error(error)
    end
  end

  def create(_, _, _), do: {:error, :invalid_room_id}

  @doc "Fetch room row once (owner + visibility + scene)."
  @spec get(room_id()) ::
          {:ok, Room.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def get(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{} = room} -> {:ok, room}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :get)
      end
    end
  end

  def get(_), do: {:error, :invalid_room_id}

  defp random_room_id do
    Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
  end

  @doc """
  Validate `room_id` and confirm the room was created (owner already assigned).
  Does not create rooms.
  """
  @spec require_existing(room_id()) ::
          :ok | {:error, :invalid_room_id | :not_found | storage_error()}
  def require_existing(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{}} -> :ok
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :require_existing)
      end
    end
  end

  def require_existing(_), do: {:error, :invalid_room_id}

  @doc "Return the owner sub for an existing room."
  @spec owner(room_id()) ::
          {:ok, owner_sub()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def owner(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{owner_sub: sub}} -> {:ok, sub}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :owner)
      end
    end
  end

  def owner(_), do: {:error, :invalid_room_id}

  @doc "True when `sub` owns the room. Distinguishes not-found / storage from mismatch."
  @spec ensure_owner(room_id(), owner_sub()) ::
          :ok | {:error, :not_found | :forbidden_host | :invalid_room_id | storage_error()}
  def ensure_owner(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{owner_sub: ^sub}} ->
          :ok

        {:ok, %Room{}} ->
          {:error, :forbidden_host}

        {:ok, nil} ->
          {:error, :not_found}

        {:error, error} ->
          map_read_error(error, :ensure_owner)
      end
    end
  end

  def ensure_owner(_, _), do: {:error, :invalid_room_id}

  @doc "Return the scene definition for an existing room (x0027)."
  @spec scene(room_id()) ::
          {:ok, map()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def scene(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{scene: scene}} when is_map(scene) -> {:ok, scene}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :scene)
      end
    end
  end

  def scene(_), do: {:error, :invalid_room_id}

  @doc "Return visibility for an existing room (x0034)."
  @spec visibility(room_id()) ::
          {:ok, String.t()} | {:error, :invalid_room_id | :not_found | storage_error()}
  def visibility(room_id) when is_binary(room_id) do
    with :ok <- validate_room_id(room_id) do
      case Room.get_by_room_id(room_id) do
        {:ok, %Room{visibility: vis}} when is_binary(vis) -> {:ok, vis}
        {:ok, nil} -> {:error, :not_found}
        {:error, error} -> map_read_error(error, :visibility)
      end
    end
  end

  def visibility(_), do: {:error, :invalid_room_id}

  @doc "Owner updates visibility (x0034)."
  @spec set_visibility(room_id(), owner_sub(), String.t()) ::
          {:ok, String.t()}
          | {:error,
             :invalid_room_id
             | :not_found
             | :forbidden_host
             | :invalid_visibility
             | storage_error()}
  def set_visibility(room_id, sub, visibility)
      when is_binary(room_id) and is_binary(sub) and is_binary(visibility) do
    with {:ok, vis} <- World.Visibility.parse(visibility),
         {:ok, %Room{} = room} <- get(room_id),
         :ok <- owner_matches(room, sub) do
      case Room.set_visibility(room, %{visibility: vis}) do
        {:ok, %{visibility: stored}} -> {:ok, stored}
        {:error, error} -> storage_error(error, :set_visibility)
      end
    end
  end

  def set_visibility(_, _, _), do: {:error, :invalid_room_id}

  defp owner_matches(%Room{owner_sub: owner}, sub) when owner == sub, do: :ok
  defp owner_matches(%Room{}, _), do: {:error, :forbidden_host}


  defp insert_room(room_id, owner_sub, visibility) do
    Room.create(%{
      room_id: room_id,
      owner_sub: owner_sub,
      scene: DemoScene.build(),
      visibility: visibility
    })
  end

  defp map_create_error(error) do
    if unique_violation?(error) do
      {:error, :already_exists}
    else
      storage_error(error, :create)
    end
  end

  defp map_read_error(error, where) do
    if not_found_error?(error) do
      {:error, :not_found}
    else
      storage_error(error, where)
    end
  end

  defp storage_error(error, where) do
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

  defp unique_violation?(error) do
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

  defp validate_room_id(room_id), do: World.RoomId.validate(room_id)
end
