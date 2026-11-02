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
    # Intentional no-op (same as rooms_wire_cap): SQLite DROP COLUMN support varies;
    # rollback of this slice is "restore DB from backup", not a reversible migration.
    # The `room_key` column remains after a down.
    :ok
  end
end
