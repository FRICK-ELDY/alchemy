defmodule World.Rooms do
  @moduledoc """
  In-process ephemeral room registry (Slice B — no DB).
  """

  use GenServer

  @type room_id :: String.t()

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec create(room_id() | nil) :: {:ok, room_id()} | {:error, :invalid_room_id | :already_exists}
  def create(nil), do: create(random_room_id())

  def create(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:create, room_id})
  end

  def create(_), do: {:error, :invalid_room_id}

  defp random_room_id do
    Base.encode16(:crypto.strong_rand_bytes(8), case: :lower)
  end

  @spec ensure(room_id()) :: {:ok, room_id()} | {:error, :invalid_room_id}
  def ensure(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:ensure, room_id})
  end

  def ensure(_), do: {:error, :invalid_room_id}

  @spec exists?(room_id()) :: boolean()
  def exists?(room_id) when is_binary(room_id) do
    GenServer.call(__MODULE__, {:exists?, room_id})
  end

  def exists?(_), do: false

  @impl true
  def init(_opts), do: {:ok, MapSet.new()}

  @impl true
  def handle_call({:create, room_id}, _from, rooms) do
    case validate_room_id(room_id) do
      :ok ->
        if MapSet.member?(rooms, room_id) do
          {:reply, {:error, :already_exists}, rooms}
        else
          {:reply, {:ok, room_id}, MapSet.put(rooms, room_id)}
        end

      {:error, reason} ->
        {:reply, {:error, reason}, rooms}
    end
  end

  @impl true
  def handle_call({:ensure, room_id}, _from, rooms) do
    case validate_room_id(room_id) do
      :ok ->
        {:reply, {:ok, room_id}, MapSet.put(rooms, room_id)}

      {:error, reason} ->
        {:reply, {:error, reason}, rooms}
    end
  end

  @impl true
  def handle_call({:exists?, room_id}, _from, rooms) do
    {:reply, MapSet.member?(rooms, room_id), rooms}
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
