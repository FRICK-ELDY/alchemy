defmodule Assets.Repo.Migrations.BlobGrantsAndAccess do
  use Ecto.Migration

  def up do
    alter table(:cas_objects) do
      add :access, :string, null: false, default: "authenticated"
    end

    # Integrity of blake3 → cas_objects is an application invariant (no FK):
    # SQLite foreign_keys pragma is not enabled project-wide.
    create table(:blob_grants, primary_key: false) do
      add :grantee_sub, :string, primary_key: true, null: false
      add :blake3, :string, primary_key: true, null: false
      add :grant_type, :string, null: false, default: "owned"

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:blob_grants, [:blake3])

    execute("""
    INSERT INTO blob_grants (grantee_sub, blake3, grant_type, inserted_at)
    SELECT owner_sub, blake3, 'owned', inserted_at FROM blob_refs
    """)
  end

  def down do
    drop table(:blob_grants)

    alter table(:cas_objects) do
      remove :access
    end
  end
end
