defmodule Assets.Repo.Migrations.CasObjectsAndBlobRefs do
  use Ecto.Migration

  def up do
    create table(:cas_objects, primary_key: false) do
      add :blake3, :string, primary_key: true, null: false
      add :size, :integer, null: false
      add :mime, :string, null: false, default: "application/octet-stream"

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    execute("""
    INSERT INTO cas_objects (blake3, size, mime, inserted_at)
    SELECT blake3, size, mime, inserted_at FROM blob_refs
    """)

    rename table(:blob_refs), to: table(:blob_refs_legacy)

    # Integrity of blake3 → cas_objects is an application invariant (no FK):
    # SQLite foreign_keys pragma is not enabled project-wide.
    create table(:blob_refs, primary_key: false) do
      add :owner_sub, :string, primary_key: true, null: false
      add :blake3, :string, primary_key: true, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:blob_refs, [:blake3])

    execute("""
    INSERT INTO blob_refs (owner_sub, blake3, inserted_at)
    SELECT owner_sub, blake3, inserted_at FROM blob_refs_legacy
    """)

    drop table(:blob_refs_legacy)

    alter table(:quota_ledgers) do
      add :reserved_bytes, :integer, null: false, default: 0
    end
  end

  def down do
    # Best-effort only: after cross-owner sharing, multiple blob_refs share one
    # blake3 and cannot fit the legacy blake3 PK. Prefer ecto.drop / forward migrate.
    alter table(:quota_ledgers) do
      remove :reserved_bytes
    end

    rename table(:blob_refs), to: table(:blob_refs_new)

    create table(:blob_refs, primary_key: false) do
      add :blake3, :string, primary_key: true, null: false
      add :size, :integer, null: false
      add :mime, :string, null: false, default: "application/octet-stream"
      add :owner_sub, :string, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:blob_refs, [:owner_sub])

    execute("""
    INSERT INTO blob_refs (blake3, size, mime, owner_sub, inserted_at)
    SELECT r.blake3, o.size, o.mime, r.owner_sub, r.inserted_at
    FROM blob_refs_new AS r
    INNER JOIN cas_objects AS o ON o.blake3 = r.blake3
    GROUP BY r.blake3
    """)

    drop table(:blob_refs_new)
    drop table(:cas_objects)
  end
end
