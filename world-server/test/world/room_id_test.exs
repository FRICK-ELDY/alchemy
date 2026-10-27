defmodule World.RoomIdTest do
  use ExUnit.Case, async: true

  alias World.RoomId

  # Keep in sync with protocol `validate_room_id_shared_vectors` (x002e).
  test "shared vectors with protocol validate_room_id" do
    for id <- ["local", "alpha", "a", "A_b-9", String.duplicate("x", 64)] do
      assert :ok = RoomId.validate(id)
      assert RoomId.valid?(id)
    end

    assert {:error, :invalid_room_id} = RoomId.validate("")
    assert {:error, :invalid_room_id} = RoomId.validate("a/b")
    assert {:error, :invalid_room_id} = RoomId.validate(String.duplicate("x", 65))

    for bad <- ["部屋", "a b", "a*b", "a.b", "a@b"] do
      assert {:error, :invalid_room_id} = RoomId.validate(bad), "reject #{inspect(bad)}"
    end
  end
end
