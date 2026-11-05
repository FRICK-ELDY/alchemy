defmodule Assets.Repo.Migrations.QuotaReservationsAndOrphanCandidates do
  use Ecto.Migration

  def up do
    create table(:quota_reservations, primary_key: false) do
      add :id, :binary_id, primary_key: true
      add :owner_sub, :string, null: false
      add :blake3, :string, null: false
      add :bytes, :integer, null: false
      add :reserved_at, :utc_datetime_usec, null: false
    end

    create unique_index(:quota_reservations, [:owner_sub, :blake3])
    create index(:quota_reservations, [:reserved_at])
    create index(:quota_reservations, [:blake3])

    create table(:cas_orphan_candidates, primary_key: false) do
      add :blake3, :string, primary_key: true, null: false
      add :noticed_at, :utc_datetime_usec, null: false
    end

    create index(:cas_orphan_candidates, [:noticed_at])

    # Pre-x0074 rows have reserved_bytes without blake3 tracking — reclaim by clamp.
    execute("UPDATE quota_ledgers SET reserved_bytes = 0")
  end

  def down do
    drop table(:cas_orphan_candidates)
    drop table(:quota_reservations)
  end
end
