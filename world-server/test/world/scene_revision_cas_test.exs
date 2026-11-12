defmodule World.SceneRevisionCasTest do
  @moduledoc """
  x0082: append+delete CAS contracts.

  True multi-process concurrency is not used here: DataCase owns a *shared*
  Ecto sandbox connection (`async: false`), and concurrent `Task.async`
  callers interleave queries on that one connection — which flakes under CI
  load without proving production locking. Orderings below still require
  conflict + retry for both mutations to land.

  Assertions key on blake3 (not entity_id): after delete, the next append may
  reuse the freed max+1 id (`next_entity_id/1`).
  """
  use World.DataCase, async: false

  alias World.Rooms

  @owner "11111111-1111-1111-1111-111111111111"
  @append_blake3 String.duplicate("a", 64)
  @seed_blake3 String.duplicate("b", 64)

  test "append then delete retry: both mutations land" do
    {room_id, seed_id, base_rev} = seeded_room!("cas-append-first")

    assert {:ok, after_append} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @append_blake3}, base_rev, 0)

    assert after_append["revision"] == base_rev + 1

    assert {:error, :scene_revision_conflict} =
             Rooms.delete_scene_entry(room_id, seed_id, base_rev, 0)

    assert {:ok, final} = Rooms.delete_scene_entry(room_id, seed_id, after_append["revision"], 0)

    assert final["revision"] == base_rev + 2
    refute Enum.any?(final["entries"], fn e -> e["blake3"] == @seed_blake3 end)
    assert Enum.any?(final["entries"], fn e -> e["blake3"] == @append_blake3 end)
  end

  test "delete then append retry: both mutations land" do
    {room_id, seed_id, base_rev} = seeded_room!("cas-delete-first")

    assert {:ok, after_delete} = Rooms.delete_scene_entry(room_id, seed_id, base_rev, 0)
    assert after_delete["revision"] == base_rev + 1
    refute Enum.any?(after_delete["entries"], fn e -> e["blake3"] == @seed_blake3 end)

    assert {:error, :scene_revision_conflict} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @append_blake3}, base_rev, 0)

    assert {:ok, final} =
             Rooms.append_scene_entry(
               room_id,
               %{"blake3" => @append_blake3},
               after_delete["revision"],
               0
             )

    assert final["revision"] == base_rev + 2
    refute Enum.any?(final["entries"], fn e -> e["blake3"] == @seed_blake3 end)
    assert Enum.any?(final["entries"], fn e -> e["blake3"] == @append_blake3 end)
  end

  test "same expected_revision: second writer gets scene_revision_conflict" do
    {room_id, seed_id, rev} = seeded_room!("cas-one-loses")

    assert {:ok, _} = Rooms.append_scene_entry(room_id, %{"blake3" => @append_blake3}, rev, 0)

    assert {:error, :scene_revision_conflict} =
             Rooms.delete_scene_entry(room_id, seed_id, rev, 0)

    assert {:ok, final} = Rooms.scene(room_id)
    assert final["revision"] == rev + 1
    assert Enum.any?(final["entries"], fn e -> e["blake3"] == @seed_blake3 end)
    assert Enum.any?(final["entries"], fn e -> e["blake3"] == @append_blake3 end)
  end

  defp seeded_room!(room_id) do
    assert {:ok, ^room_id, _, _} = Rooms.create(room_id, @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @seed_blake3}, scene0["revision"], 0)

    seed_id = List.last(scene1["entries"])["entity_id"]
    {room_id, seed_id, scene1["revision"]}
  end
end
