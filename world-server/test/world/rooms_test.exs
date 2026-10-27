defmodule World.RoomsTest do
  use World.DataCase, async: false

  alias World.Rooms
  alias World.Rooms.Room

  @owner "22222222-2222-2222-2222-222222222222"

  test "create persists owner and demo scene" do
    assert {:ok, "persist-a", "public"} = Rooms.create("persist-a", @owner)
    assert {:ok, @owner} = Rooms.owner("persist-a")
    assert {:ok, scene} = Rooms.scene("persist-a")
    assert scene["revision"] == 1
    assert is_list(scene["entries"])
    assert length(scene["entries"]) == 14
  end

  test "create rejects duplicate room_id" do
    assert {:ok, _, _} = Rooms.create("dup-room", @owner)
    assert {:error, :already_exists} = Rooms.create("dup-room", @owner)
  end

  test "create rejects invalid room_id" do
    assert {:error, :invalid_room_id} = Rooms.create("bad/id", @owner)
    assert {:error, :missing_owner} = Rooms.create("ok-id", "")
  end

  test "require_existing and scene return not_found for missing rooms" do
    assert {:error, :not_found} = Rooms.require_existing("ghost-room")
    assert {:error, :not_found} = Rooms.scene("ghost-room")
    assert {:error, :not_found} = Rooms.owner("ghost-room")
    assert {:error, :not_found} = Rooms.ensure_owner("ghost-room", @owner)
  end

  test "ensure_owner distinguishes forbidden from not_found" do
    assert {:ok, _, _} = Rooms.create("owned-room", @owner)
    assert :ok = Rooms.ensure_owner("owned-room", @owner)
    assert {:error, :forbidden_host} = Rooms.ensure_owner("owned-room", "other-sub")
  end

  test "room survives beyond in-memory process lifetime (DB read)" do
    assert {:ok, "survive", _} = Rooms.create("survive", @owner)

    # Direct Ash read proves the row is in SQLite, not a GenServer map.
    assert {:ok, %Room{owner_sub: @owner, scene: scene}} = Room.get_by_room_id("survive")
    assert scene["locomotion"]["mode"] == "noclip"
  end

  test "duplicate create maps to already_exists via primary key" do
    assert {:ok, _, _} = Rooms.create("pk-dup", @owner)
    # Bypass ensure_absent-style races: second Ash insert hits PK uniqueness.
    assert {:error, :already_exists} = Rooms.create("pk-dup", @owner)
  end

  test "create and reads return storage_error when repo is down" do
    assert {:ok, _, _} = Rooms.create("before-down", @owner)
    stop_repo!()

    assert {:error, :storage_error} = Rooms.create("during-down", @owner)
    assert {:error, :storage_error} = Rooms.require_existing("before-down")
    assert {:error, :storage_error} = Rooms.owner("before-down")
    assert {:error, :storage_error} = Rooms.ensure_owner("before-down", @owner)
    assert {:error, :storage_error} = Rooms.scene("before-down")
  after
    restart_repo!()
  end

  defp stop_repo! do
    case Supervisor.terminate_child(World.Supervisor, World.Repo) do
      :ok -> :ok
      {:error, :not_found} -> :ok
    end
  end

  defp restart_repo! do
    case Supervisor.restart_child(World.Supervisor, World.Repo) do
      {:ok, _} -> :ok
      {:ok, _, _} -> :ok
      {:error, {:already_started, _}} -> :ok
      {:error, :running} -> :ok
      other -> flunk("failed to restart World.Repo: #{inspect(other)}")
    end
  end
end
