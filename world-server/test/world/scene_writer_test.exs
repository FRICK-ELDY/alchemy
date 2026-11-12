defmodule World.SceneWriterTest do
  @moduledoc "x009d: Rooms.ensure_scene_writer — owner ∪ editor_subs"
  use World.DataCase, async: false

  alias World.Repo
  alias World.Rooms

  test "ensure_scene_writer allows owner and listed editor only" do
    owner = "c3c3c3c3-c3c3-c3c3-c3c3-c3c3c3c3c3c3"
    editor = "d4d4d4d4-d4d4-d4d4-d4d4-d4d4d4d4d4d4"
    stranger = "e5e5e5e5-e5e5-e5e5-e5e5-e5e5e5e5e5e5"
    room_id = "sw-1"

    assert {:ok, ^room_id, "public", _} = Rooms.create(room_id, owner, "public")
    assert :ok = Rooms.ensure_scene_writer(room_id, owner)
    assert {:error, :forbidden_scene_writer} = Rooms.ensure_scene_writer(room_id, editor)
    assert {:error, :forbidden_scene_writer} = Rooms.ensure_scene_writer(room_id, stranger)

    assert :ok = Rooms.set_editor_subs(room_id, owner, [editor])
    assert :ok = Rooms.ensure_scene_writer(room_id, owner)
    assert :ok = Rooms.ensure_scene_writer(room_id, editor)
    assert {:error, :forbidden_scene_writer} = Rooms.ensure_scene_writer(room_id, stranger)

    assert {:error, :forbidden_host} = Rooms.set_editor_subs(room_id, stranger, [stranger])

    # Corrupt JSON → treat as empty editors (fail-closed for non-owner).
    assert {:ok, _} =
             Repo.query("UPDATE rooms SET editor_subs = ? WHERE room_id = ?", [
               "not-json",
               room_id
             ])

    assert :ok = Rooms.ensure_scene_writer(room_id, owner)
    assert {:error, :forbidden_scene_writer} = Rooms.ensure_scene_writer(room_id, editor)

    too_many =
      for i <- 1..65 do
        "00000000-0000-0000-0000-" <> String.pad_leading(Integer.to_string(i), 12, "0")
      end

    assert {:error, :invalid_editors} = Rooms.set_editor_subs(room_id, owner, too_many)
  end
end
