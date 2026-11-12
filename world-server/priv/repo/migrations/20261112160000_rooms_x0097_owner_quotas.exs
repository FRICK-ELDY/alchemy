defmodule World.Repo.Migrations.RoomsX0097OwnerQuotas do
  @moduledoc """
  x0097: per-owner room count ledger for strict create cap (conditional UPDATE).

  Backfills from existing `rooms` so pre-migration inventories match quota rows.
  """
  use Ecto.Migration

  def up do
    create table(:room_owner_quotas, primary_key: false) do
      add :owner_sub, :text, primary_key: true, null: false
      add :room_count, :integer, null: false, default: 0

      timestamps(type: :utc_datetime_usec)
    end

    # Prefer fractional seconds to match `utc_datetime_usec` (CURRENT_TIMESTAMP is seconds).
    execute("""
    INSERT INTO room_owner_quotas (owner_sub, room_count, inserted_at, updated_at)
    SELECT owner_sub,
           COUNT(*),
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    FROM rooms
    GROUP BY owner_sub
    """)
  end

  def down do
    drop table(:room_owner_quotas)
  end
end
