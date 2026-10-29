defmodule World.Repo.Migrations.RoomsWireCap do
  @moduledoc """
  x004e: per-room Zenoh key channel secret (`wire_cap`).
  """
  use Ecto.Migration

  def up do
    # Existing rows get a backfilled secret; new creates set wire_cap in Ash create.
    execute("""
    ALTER TABLE rooms ADD COLUMN wire_cap TEXT NOT NULL DEFAULT '';
    """)

    execute("""
    UPDATE rooms
    SET wire_cap = lower(hex(randomblob(16)))
    WHERE wire_cap = '' OR wire_cap IS NULL;
    """)
  end

  def down do
    # Intentional no-op: SQLite versions differ on DROP COLUMN; rollback of this slice
    # is "restore DB from backup", not a reversible migration.
    :ok
  end
end
