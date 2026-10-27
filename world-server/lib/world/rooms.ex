defmodule World.Rooms do
  @moduledoc """
  Room ledger facade (x0028).

  Guarantees: create / owner / scene backed by Ash + SQLite (survives restart).
  Non-guarantees: UGC scene edits; multi-node replication; Postgres (swap path documented).

  x0021: `owner_sub` set only via `create/2`. Token issuance never creates a room.
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

  @spec create(room_id() | nil, owner_sub()) ::
          {:ok, room_id()}
          | {:error, :invalid_room_id | :already_exists | :missing_owner | storage_error()}
  def create(nil, owner_sub), do: create(random_room_id(), owner_sub)

  def create(_room_id, owner_sub)
      when not is_binary(owner_sub) or owner_sub == "" do
    {:error, :missing_owner}
  end

  def create(room_id, owner_sub) when is_binary(room_id) and is_binary(owner_sub) do
    with :ok <- validate_room_id(room_id),
         {:ok, %Room{room_id: id}} <- insert_room(room_id, owner_sub) do
      {:ok, id}
    else
      {:error, :invalid_room_id} = err -> err
      {:error, :already_exists} = err -> err
      {:error, :storage_error} = err -> err
      {:error, error} -> map_create_error(error)
    end
  end

  def create(_, _), do: {:error, :invalid_room_id}

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

  defp insert_room(room_id, owner_sub) do
    Room.create(%{
      room_id: room_id,
      owner_sub: owner_sub,
      scene: DemoScene.build()
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

  defp validate_room_id(room_id) do
    cond do
      room_id == "" ->
        {:error, :invalid_room_id}

      String.contains?(room_id, "/") ->
        {:error, :invalid_room_id}

      byte_size(room_id) > 64 ->
        {:error, :invalid_room_id}

      not Regex.match?(~r/^[A-Za-z0-9_-]+$/, room_id) ->
        {:error, :invalid_room_id}

      true ->
        :ok
    end
  end
end
