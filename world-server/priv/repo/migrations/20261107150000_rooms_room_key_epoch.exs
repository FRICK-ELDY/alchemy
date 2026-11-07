defmodule World.Repo.Migrations.RoomsRoomKeyEpoch do
  @moduledoc """
  x007e: monotonic `room_key_epoch` for revoke-driven Room key rotation.
  Current key material remains in `room_key`; previous generation is memory-only (RoomKeyGrace).
  """
  use Ecto.Migration

  def up do
    execute("""
    ALTER TABLE rooms ADD COLUMN room_key_epoch INTEGER NOT NULL DEFAULT 0;
    """)
  end

  def down do
    # Intentional no-op (SQLite DROP COLUMN / slice rollback = restore from backup).
    :ok
  end
end
