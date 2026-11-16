defmodule Assets.Repo.Migrations.OriginCertV2Origins do
  use Ecto.Migration

  @moduledoc """
  x00ca: Origin Cert v2 columns + `vcs_blob_origins` first-seen ledger.

  v1 cert rows (if any) are DELETE'd — x00c9 already wiped UUID-era certs;
  remaining v1 message shapes are not verified after this migration.
  `vcs_certs.target_kind` is replaced by `cert_kind` (commit | release).

  Unique: one Authorship per commit_hash; one Release per repo+tag (partial indexes).
  """

  def up do
    execute("DELETE FROM vcs_certs")

    # SQLite: drop + recreate is simpler than ALTER REMOVE for target_kind rename.
    drop table(:vcs_certs)

    create table(:vcs_certs, primary_key: false) do
      add :cert_id, :string, primary_key: true, null: false
      add :subject_sub, :string, null: false
      # commit | release (Authorship / Release)
      add :cert_kind, :string, null: false
      # Always the bound commit_hash
      add :target_id, :string, null: false
      # Release only
      add :repo_id, :string, null: true
      add :tag_name, :string, null: true
      add :issued_at_ms, :integer, null: false
      add :kid, :string, null: false
      add :message_version, :string, null: false
      # blake3 hex of canonical cert message
      add :target_hash, :string, null: false
      # Base64-encoded Ed25519 signature
      add :signature, :string, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_certs, [:target_id])
    create index(:vcs_certs, [:subject_sub])
    create index(:vcs_certs, [:cert_kind])

    create unique_index(:vcs_certs, [:target_id],
             name: :vcs_certs_authorship_commit_index,
             where: "cert_kind = 'commit'"
           )

    create unique_index(:vcs_certs, [:repo_id, :tag_name],
             name: :vcs_certs_release_repo_tag_index,
             where: "cert_kind = 'release'"
           )

    create table(:vcs_blob_origins, primary_key: false) do
      add :blake3, :string, primary_key: true, null: false
      add :first_commit_hash, :string, null: false
      add :author_sub, :string, null: false
      add :first_seen_at, :utc_datetime_usec, null: false
    end

    create index(:vcs_blob_origins, [:author_sub])
    create index(:vcs_blob_origins, [:first_commit_hash])
  end

  def down do
    drop table(:vcs_blob_origins)
    drop table(:vcs_certs)

    create table(:vcs_certs, primary_key: false) do
      add :cert_id, :string, primary_key: true, null: false
      add :subject_sub, :string, null: false
      add :target_kind, :string, null: false
      add :target_id, :string, null: false
      add :target_hash, :string, null: false
      add :signature, :string, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_certs, [:target_id])
    create index(:vcs_certs, [:subject_sub])
  end
end
