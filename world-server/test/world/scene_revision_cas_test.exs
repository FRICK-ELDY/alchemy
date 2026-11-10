defmodule World.SceneRevisionCasTest do
  @moduledoc "x0082: concurrent append+delete must not drop either mutation after retry"
  use World.DataCase, async: false

  alias World.Rooms

  @owner "11111111-1111-1111-1111-111111111111"
  @append_blake3 String.duplicate("a", 64)
  @seed_blake3 String.duplicate("b", 64)

  test "concurrent append and delete both land after CAS retry" do
    assert {:ok, room_id, _, _} = Rooms.create("cas-race", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @seed_blake3}, scene0["revision"])

    seed_id = List.last(scene1["entries"])["entity_id"]
    base_rev = scene1["revision"]

    append_task =
      Task.async(fn ->
        mutate_with_retry(room_id, base_rev, fn rev ->
          Rooms.append_scene_entry(room_id, %{"blake3" => @append_blake3}, rev)
        end)
      end)

    delete_task =
      Task.async(fn ->
        mutate_with_retry(room_id, base_rev, fn rev ->
          Rooms.delete_scene_entry(room_id, seed_id, rev)
        end)
      end)

    assert {:ok, _} = Task.await(append_task, 5_000)
    assert {:ok, _} = Task.await(delete_task, 5_000)

    assert {:ok, final} = Rooms.scene(room_id)
    assert final["revision"] == base_rev + 2
    refute Enum.any?(final["entries"], fn e -> e["entity_id"] == seed_id end)
    assert Enum.any?(final["entries"], fn e -> e["blake3"] == @append_blake3 end)
  end

  test "same expected_revision loses one writer to scene_revision_conflict" do
    assert {:ok, room_id, _, _} = Rooms.create("cas-one-loses", @owner)
    assert {:ok, scene0} = Rooms.scene(room_id)

    assert {:ok, scene1} =
             Rooms.append_scene_entry(room_id, %{"blake3" => @seed_blake3}, scene0["revision"])

    seed_id = List.last(scene1["entries"])["entity_id"]
    rev = scene1["revision"]

    results =
      [
        Task.async(fn ->
          Rooms.append_scene_entry(room_id, %{"blake3" => @append_blake3}, rev)
        end),
        Task.async(fn -> Rooms.delete_scene_entry(room_id, seed_id, rev) end)
      ]
      |> Enum.map(&Task.await(&1, 5_000))

    oks = Enum.count(results, &match?({:ok, _}, &1))
    conflicts = Enum.count(results, &match?({:error, :scene_revision_conflict}, &1))
    assert oks == 1
    assert conflicts == 1
  end

  defp mutate_with_retry(room_id, rev, fun, attempts_left \\ 8)

  defp mutate_with_retry(_room_id, _rev, _fun, 0), do: {:error, :scene_revision_conflict}

  defp mutate_with_retry(room_id, rev, fun, attempts_left) do
    case fun.(rev) do
      {:ok, _} = ok ->
        ok

      {:error, :scene_revision_conflict} ->
        assert {:ok, fresh} = Rooms.scene(room_id)
        mutate_with_retry(room_id, fresh["revision"], fun, attempts_left - 1)

      other ->
        other
    end
  end
end
