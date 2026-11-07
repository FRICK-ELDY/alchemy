defmodule World.RoomsTest do
  use World.DataCase, async: false

  alias Ecto.Adapters.SQL.Sandbox
  alias World.Rooms
  alias World.Rooms.Room

  @owner "22222222-2222-2222-2222-222222222222"

  test "create persists owner and demo scene" do
    assert {:ok, "persist-a", "public", wire_cap} = Rooms.create("persist-a", @owner)
    assert is_binary(wire_cap) and wire_cap != ""
    assert {:ok, @owner} = Rooms.owner("persist-a")
    assert {:ok, scene} = Rooms.scene("persist-a")
    assert scene["revision"] == 1
    assert is_list(scene["entries"])
    assert length(scene["entries"]) == 14
  end

  test "create stores room_key hex and token encoding is 32 bytes (x0069)" do
    assert {:ok, "rk-room", _, _} = Rooms.create("rk-room", @owner)
    assert {:ok, %Room{room_key: hex, room_key_epoch: 0}} = Room.get_by_room_id("rk-room")
    assert byte_size(hex) == 64
    assert {:ok, b64} = Rooms.room_key_for_token(hex)
    assert {:ok, raw} = Base.url_decode64(b64, padding: false)
    assert byte_size(raw) == 32
  end

  test "visibility narrow rotates room_key epoch and parks prev in grace (x007e)" do
    alias World.Rooms.RoomKeyGrace

    assert {:ok, "rk-narrow", _, _} = Rooms.create("rk-narrow", @owner, "public")
    assert {:ok, %Room{room_key: key0, room_key_epoch: 0}} = Room.get_by_room_id("rk-narrow")

    assert {:ok, "friends"} = Rooms.set_visibility("rk-narrow", @owner, "friends")

    assert {:ok, %Room{room_key: key1, room_key_epoch: 1, visibility: "friends"}} =
             Room.get_by_room_id("rk-narrow")

    assert key1 != key0
    assert {:ok, %{prev_key: ^key0, prev_epoch: 0}} = RoomKeyGrace.get("rk-narrow")

    # Widen does not rotate.
    assert {:ok, "public"} = Rooms.set_visibility("rk-narrow", @owner, "public")
    assert {:ok, %Room{room_key: ^key1, room_key_epoch: 1}} = Room.get_by_room_id("rk-narrow")
  end

  test "owner rotate_room_key bumps epoch without visibility change (x007e)" do
    alias World.Rooms.RoomKeyGrace

    assert {:ok, "rk-rot", _, _} = Rooms.create("rk-rot", @owner)

    assert {:ok, %Room{room_key: key0, room_key_epoch: 0, visibility: vis}} =
             Room.get_by_room_id("rk-rot")

    assert {:ok, 1} = Rooms.rotate_room_key("rk-rot", @owner)

    assert {:ok, %Room{room_key: key1, room_key_epoch: 1, visibility: ^vis}} =
             Room.get_by_room_id("rk-rot")

    assert key1 != key0
    assert {:ok, %{prev_key: ^key0, prev_epoch: 0}} = RoomKeyGrace.get("rk-rot")
    assert {:error, :forbidden_host} = Rooms.rotate_room_key("rk-rot", "other-sub")
  end

  test "rotate_due_room_keys rotates stale rooms (x007e periodic)" do
    assert {:ok, "rk-due", _, _} = Rooms.create("rk-due", @owner)
    assert {:ok, %Room{room_key_epoch: 0} = room} = Room.get_by_room_id("rk-due")
    stale = DateTime.add(DateTime.utc_now(), -48 * 3600, :second) |> DateTime.truncate(:second)

    assert {:ok, _} =
             Room.rotate_room_key(room, %{
               room_key: room.room_key,
               room_key_epoch: room.room_key_epoch,
               room_key_rotated_at: stale
             })

    assert {0, 0} = Rooms.rotate_due_room_keys(0)
    {ok_n, err_n} = Rooms.rotate_due_room_keys(24)
    assert err_n == 0
    assert ok_n >= 1
    assert {:ok, %Room{room_key_epoch: epoch}} = Room.get_by_room_id("rk-due")
    assert epoch >= 1
  end

  test "create rejects duplicate room_id" do
    assert {:ok, _, _, _} = Rooms.create("dup-room", @owner)
    assert {:error, :already_exists} = Rooms.create("dup-room", @owner)
  end

  test "create rejects invalid room_id" do
    assert {:error, :invalid_room_id} = Rooms.create("bad/id", @owner)
    assert {:error, :missing_owner} = Rooms.create("ok-id", "")
  end

  test "create rejects when owner room cap is exceeded (x0071)" do
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 1)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    assert {:ok, "cap-1", _, _} = Rooms.create("cap-1", @owner)
    assert {:error, :owner_room_cap_exceeded} = Rooms.create("cap-2", @owner)
  end

  test "create returns storage_error when max_rooms_per_owner is invalid (x0071)" do
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 0)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    assert {:error, :storage_error} = Rooms.create("bad-cap-room", @owner)
  end

  test "require_existing and scene return not_found for missing rooms" do
    assert {:error, :not_found} = Rooms.require_existing("ghost-room")
    assert {:error, :not_found} = Rooms.scene("ghost-room")
    assert {:error, :not_found} = Rooms.owner("ghost-room")
    assert {:error, :not_found} = Rooms.ensure_owner("ghost-room", @owner)
  end

  test "ensure_owner distinguishes forbidden from not_found" do
    assert {:ok, _, _, _} = Rooms.create("owned-room", @owner)
    assert :ok = Rooms.ensure_owner("owned-room", @owner)
    assert {:error, :forbidden_host} = Rooms.ensure_owner("owned-room", "other-sub")
  end

  test "room survives beyond in-memory process lifetime (DB read)" do
    assert {:ok, "survive", _, _} = Rooms.create("survive", @owner)

    # Direct Ash read proves the row is in SQLite, not a GenServer map.
    assert {:ok, %Room{owner_sub: @owner, scene: scene}} = Room.get_by_room_id("survive")
    assert scene["locomotion"]["mode"] == "noclip"
  end

  test "duplicate create maps to already_exists via primary key" do
    assert {:ok, _, _, _} = Rooms.create("pk-dup", @owner)
    # Bypass ensure_absent-style races: second Ash insert hits PK uniqueness.
    assert {:error, :already_exists} = Rooms.create("pk-dup", @owner)
  end

  test "claim_host_generation reuses same pub and bumps on handoff (x003a)" do
    assert {:ok, "gen-room", _, _} = Rooms.create("gen-room", @owner)
    pub_a = :crypto.strong_rand_bytes(32)
    pub_b = :crypto.strong_rand_bytes(32)

    assert {:ok, 1} = Rooms.claim_host_generation("gen-room", pub_a)
    assert {:ok, 1} = Rooms.claim_host_generation("gen-room", pub_a)
    assert {:ok, 2} = Rooms.claim_host_generation("gen-room", pub_b)
    assert {:ok, 2} = Rooms.claim_host_generation("gen-room", pub_b)

    assert :ok = Rooms.match_claimed_host("gen-room", pub_b, 2)
    assert {:error, :stale_host_attestation} = Rooms.match_claimed_host("gen-room", pub_a, 1)
  end

  test "claim_host_generation CAS rejects stale expected row (x003a dual-host race)" do
    assert {:ok, "cas-room", _, _} = Rooms.create("cas-room", @owner)
    pub_a = :crypto.strong_rand_bytes(32)
    pub_b = :crypto.strong_rand_bytes(32)
    assert {:ok, 1} = Rooms.claim_host_generation("cas-room", pub_a)

    # Lost race: UPDATE still expecting gen=0 / nil pub must not overwrite gen=1.
    encoded_b = Base.url_encode64(pub_b, padding: false)

    sql = """
    UPDATE rooms
    SET host_generation = 99, attested_host_pub = ?, updated_at = CURRENT_TIMESTAMP
    WHERE room_id = ? AND host_generation = 0 AND attested_host_pub IS NULL
    """

    assert {:ok, %{num_rows: 0}} = World.Repo.query(sql, [encoded_b, "cas-room"])
    assert {:ok, 1} = Rooms.claim_host_generation("cas-room", pub_a)
    assert {:ok, 2} = Rooms.claim_host_generation("cas-room", pub_b)
  end

  test "create and reads return storage_error when repo is down" do
    assert {:ok, _, _, _} = Rooms.create("before-down", @owner)
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

    # x003b: Repo restart leaves Sandbox in auto; restore manual for later DataCase owners.
    Sandbox.mode(World.Repo, :manual)
  end
end
