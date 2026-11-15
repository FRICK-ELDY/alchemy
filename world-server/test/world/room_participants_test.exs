defmodule World.RoomParticipantsTest do
  use World.DataCase, async: false

  alias World.RoomParticipants
  alias World.Rooms

  @owner "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
  @editor "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
  @stranger "cccccccc-cccc-cccc-cccc-cccccccccccc"

  setup do
    RoomParticipants.ensure_table!()
    :ok
  end

  test "editor_subs JSON path works without Room Token ledger (x00b5)" do
    room_id = "rp-editor-json"
    assert {:ok, ^room_id, _, _} = Rooms.create(room_id, @owner)
    assert :ok = Rooms.set_editor_subs(room_id, @owner, [@editor])

    # No RoomParticipants.note/3 for editor — role path must decode JSON column.
    assert RoomParticipants.participant?(room_id, @owner)
    assert RoomParticipants.participant?(room_id, @editor)
    refute RoomParticipants.participant?(room_id, @stranger)
  end

  test "List.wrap on raw editor_subs string would miss editor (regression guard)" do
    room_id = "rp-editor-raw"
    assert {:ok, ^room_id, _, _} = Rooms.create(room_id, @owner)
    assert :ok = Rooms.set_editor_subs(room_id, @owner, [@editor])
    assert {:ok, room} = Rooms.get(room_id)

    # Column is JSON text, not a list — wrapping it never matches the UUID.
    refute is_list(room.editor_subs)
    refute @editor in List.wrap(room.editor_subs)
    assert @editor in Rooms.editor_subs(room)
  end
end
