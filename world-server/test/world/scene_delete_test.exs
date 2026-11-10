defmodule World.SceneDeleteTest do
  @moduledoc "x007b / x0082: Rooms.delete_scene_entry"
  use World.DataCase, async: false

  alias World.Rooms
  alias World.Rooms.Room

  @owner "11111111-1111-1111-1111-111111111111"
  @blake3 String.duplicate("d", 64)

  test "delete_scene_entry removes mesh and bumps revision" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-mesh", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(
               room_id,
               %{
                 "blake3" => @blake3,
                 "x" => 1.0,
                 "y" => 2.0,
                 "z" => 3.0
               },
               scene0["revision"]
             )

    entity_id = List.last(scene1["entries"])["entity_id"]
    rev1 = scene1["revision"]
    n1 = length(scene1["entries"])

    assert {:ok, scene2} = Rooms.delete_scene_entry(room_id, entity_id, rev1)
    assert scene2["revision"] == rev1 + 1
    assert length(scene2["entries"]) == n1 - 1
    refute Enum.any?(scene2["entries"], fn e -> e["entity_id"] == entity_id end)
  end

  test "delete_scene_entry rejects stale expected_revision" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-stale", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @blake3}, scene0["revision"])

    entity_id = List.last(scene1["entries"])["entity_id"]

    assert {:error, :scene_revision_conflict} =
             Rooms.delete_scene_entry(room_id, entity_id, scene1["revision"] + 1)
  end

  test "delete_scene_entry rejects missing entity_id" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-missing", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:error, :entry_not_found} =
             Rooms.delete_scene_entry(room_id, 999_999, scene0["revision"])
  end

  test "delete_scene_entry rejects cuboid / empty blake3" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-box", @owner)
    assert {:ok, scene} = Rooms.scene(room_id)

    box_id =
      scene["entries"]
      |> Enum.find(fn e ->
        (e["blake3"] == "" or e["blake3"] == nil) and (e["half_extent"] || 0) > 0
      end)
      |> Map.fetch!("entity_id")

    assert {:error, :not_mesh_entry} =
             Rooms.delete_scene_entry(room_id, box_id, scene["revision"])
  end

  test "delete_scene_entry rejects uppercase blake3 as not_mesh_entry" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-upper", @owner)
    assert {:ok, %Room{} = room} = Room.get_by_room_id(room_id)

    upper = String.duplicate("A", 64)

    corrupt = %{
      "revision" => 2,
      "locomotion" => room.scene["locomotion"],
      "entries" => [
        %{
          "entity_id" => 9001,
          "blake3" => upper,
          "x" => 0.0,
          "y" => 0.0,
          "z" => 0.0,
          "half_extent" => 0.0,
          "flags" => 0
        }
      ]
    }

    assert {:ok, _} = Room.set_scene(room, %{scene: corrupt})
    assert {:error, :not_mesh_entry} = Rooms.delete_scene_entry(room_id, 9001, 2)
  end

  test "delete_scene_entry rejects duplicate entity_id as ambiguous" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-ambig", @owner)
    assert {:ok, %Room{} = room} = Room.get_by_room_id(room_id)

    mesh = %{
      "entity_id" => 42,
      "blake3" => @blake3,
      "x" => 0.0,
      "y" => 0.0,
      "z" => 0.0,
      "half_extent" => 0.0,
      "flags" => 0
    }

    corrupt = %{
      "revision" => 2,
      "locomotion" => room.scene["locomotion"],
      "entries" => [mesh, Map.put(mesh, "blake3", String.duplicate("e", 64))]
    }

    assert {:ok, _} = Room.set_scene(room, %{scene: corrupt})
    assert {:error, :ambiguous_entity_id} = Rooms.delete_scene_entry(room_id, 42, 2)
  end

  test "delete_scene_entry can remove demo coin mesh" do
    assert {:ok, room_id, _, _} = Rooms.create("delete-coin", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    coin =
      Enum.find(scene0["entries"], fn e ->
        is_binary(e["blake3"]) and byte_size(e["blake3"]) == 64
      end)

    assert coin

    assert {:ok, scene1} =
             Rooms.delete_scene_entry(room_id, coin["entity_id"], scene0["revision"])

    refute Enum.any?(scene1["entries"], fn e -> e["entity_id"] == coin["entity_id"] end)
  end
end
