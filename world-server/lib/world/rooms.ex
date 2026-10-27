defmodule World.Rooms do
  @moduledoc """
  In-process ephemeral room registry (Slice B — no DB).

  x0021: each room has an `owner_sub` set only via `create/2`. Token issuance
  never creates a room (guest/host must not become owner by side effect).
  x0027: each room stores a DemoScene definition assigned at create.
  """

  use GenServer

  alias World.DemoScene

  @type room_id :: String.t()
  @type owner_sub :: String.t()
  @type room :: %{owner_sub: owner_sub(), scene: map()}
  @type state :: %{room_id() => room()}

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec create(room_id() | nil, owner_sub()) ::
          {:ok, room_id()} | {:error, :invalid_room_id | :already_exists | :missing_owner}
  def create(nil, owner_sub), do: create(random_room_id(), owner_sub)

  def create(_room_id, owner_sub)
      when not is_binary(owner_sub) or owner_sub == "" do
    {:error, :missing_owner}
  end

  def create(room_id, owner_sub) when is_binary(room_id) and is_binary(owner_sub) do
    GenServer.call(__MODULE__, {:create, room_id, owner_sub})
  end

  def create(_, _), do: {:error, :invalid_room_id}

  defp random_room_id do
    Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
  end

  @doc """
  Validate `room_id` and confirm the room was created (owner already assigned).
  Does not create rooms.
  """
  @spec require_existing(room_id()) :: :ok | {:error, :invalid_room_id | :not_found}
  def require_existing(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:require_existing, room_id})
  end

  def require_existing(_), do: {:error, :invalid_room_id}

  @spec exists?(room_id()) :: boolean()
  def exists?(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:exists?, room_id})
  end

  def exists?(_), do: false

  @spec owner(room_id()) :: owner_sub() | nil
  def owner(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:owner, room_id})
  end

  def owner(_), do: nil

  @spec owner?(room_id(), owner_sub()) :: boolean()
  def owner?(room_id, sub) when is_binary(room_id) and is_binary(sub) do
    owner(room_id) == sub
  end

  def owner?(_, _), do: false

  @doc "Return the scene definition for an existing room (x0027)."
  @spec scene(room_id()) :: {:ok, map()} | {:error, :invalid_room_id | :not_found}
  def scene(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:scene, room_id})
  end

  def scene(_), do: {:error, :invalid_room_id}

  @impl true
  def init(_opts), do: {:ok, %{}}

  @impl true
  def handle_call({:create, room_id, owner_sub}, _from, rooms) do
    case validate_room_id(room_id) do
      :ok ->
        if Map.has_key?(rooms, room_id) do
          {:reply, {:error, :already_exists}, rooms}
        else
          room = %{owner_sub: owner_sub, scene: DemoScene.build()}
          {:reply, {:ok, room_id}, Map.put(rooms, room_id, room)}
        end

      {:error, reason} ->
        {:reply, {:error, reason}, rooms}
    end
  end

  @impl true
  def handle_call({:require_existing, room_id}, _from, rooms) do
    case validate_room_id(room_id) do
      :ok ->
        if Map.has_key?(rooms, room_id) do
          {:reply, :ok, rooms}
        else
          {:reply, {:error, :not_found}, rooms}
        end

      {:error, reason} ->
        {:reply, {:error, reason}, rooms}
    end
  end

  @impl true
  def handle_call({:exists?, room_id}, _from, rooms) do
    {:reply, Map.has_key?(rooms, room_id), rooms}
  end

  @impl true
  def handle_call({:owner, room_id}, _from, rooms) do
    owner =
      case Map.get(rooms, room_id) do
        %{owner_sub: sub} -> sub
        _ -> nil
      end

    {:reply, owner, rooms}
  end

  @impl true
  def handle_call({:scene, room_id}, _from, rooms) do
    reply =
      case validate_room_id(room_id) do
        :ok ->
          case Map.get(rooms, room_id) do
            %{scene: scene} -> {:ok, scene}
            nil -> {:error, :not_found}
          end

        {:error, reason} ->
          {:error, reason}
      end

    {:reply, reply, rooms}
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
