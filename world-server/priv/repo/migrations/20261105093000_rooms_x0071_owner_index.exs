defmodule World.Repo.Migrations.RoomsX0071OwnerIndex do
  @moduledoc """
  x0071: index for per-owner room count (cap) lookups.

  Hand-written (Ash snapshots are not the source of truth for later Room columns).
  """
  use Ecto.Migration

  def up do
    execute("""
    CREATE INDEX IF NOT EXISTS rooms_x0071_owner_sub_idx ON rooms(owner_sub)
    """)
  end

  def down do
    execute("""
    DROP INDEX IF EXISTS rooms_x0071_owner_sub_idx
    """)
  end
end
