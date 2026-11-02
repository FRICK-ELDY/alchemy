defmodule World.Repo.Migrations.RoomsRoomKey do
  @moduledoc """
  x0069: per-room AEAD key material stored as 64 lowercase hex (32 bytes).
  Token JSON exposes URL-safe base64 of the same bytes.
  """
  use Ecto.Migration

  def up do
    execute("""
    ALTER TABLE rooms ADD COLUMN room_key TEXT NOT NULL DEFAULT '';
    """)

    execute("""
    UPDATE rooms
    SET room_key = lower(hex(randomblob(32)))
    WHERE room_key = '' OR room_key IS NULL;
    """)
  end

  def down do
    :ok
  end
end
