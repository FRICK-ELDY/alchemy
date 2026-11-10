defmodule World.SceneAppendTest do
  @moduledoc "x006d / x0082: Rooms.append_scene_entry"
  use World.DataCase, async: false

  alias World.Rooms
  alias World.Rooms.Room

  @owner "11111111-1111-1111-1111-111111111111"
  @blake3 String.duplicate("c", 64)

  test "append_scene_entry bumps revision and assigns entity_id" do
    assert {:ok, room_id, _, _} = Rooms.create("append-mesh", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)
    rev0 = scene0["revision"]
    n0 = length(scene0["entries"])

    assert {:ok, scene1} =
             Rooms.append_scene_entry(
               room_id,
               %{
                 "blake3" => @blake3,
                 "x" => 1.0,
                 "y" => 2.0,
                 "z" => 3.0
               },
               rev0
             )

    assert scene1["revision"] == rev0 + 1
    assert length(scene1["entries"]) == n0 + 1
    last = List.last(scene1["entries"])
    assert last["blake3"] == @blake3
    assert last["entity_id"] >= 100
    assert last["y"] == 2.0
  end

  test "append_scene_entry rejects stale expected_revision" do
    assert {:ok, room_id, _, _} = Rooms.create("append-stale", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:error, :scene_revision_conflict} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @blake3}, scene0["revision"] + 1)
  end

  test "append_scene_entry rejects corrupt scene revision as invalid_scene_revision" do
    assert {:ok, room_id, _, _} = Rooms.create("append-corrupt-rev", @owner)
    assert {:ok, %Room{} = room} = Room.get_by_room_id(room_id)

    corrupt = Map.put(room.scene, "revision", "not-an-int")
    assert {:ok, _} = Room.set_scene(room, %{scene: corrupt})

    assert {:error, :invalid_scene_revision} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @blake3}, 1)
  end

  test "append_scene_entry rejects duplicate blake3" do
    assert {:ok, room_id, _, _} = Rooms.create("append-dup", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @blake3}, scene0["revision"])

    assert {:error, :duplicate_blake3} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @blake3}, scene1["revision"])
  end

  test "append_scene_entry rejects invalid blake3" do
    assert {:ok, room_id, _, _} = Rooms.create("append-bad", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:error, :invalid_entry} =
             Rooms.append_scene_entry(room_id, %{"blake3" => "short"}, scene0["revision"])
  end
end
