defmodule Assets.Repo.Migrations.CreateVcsTables do
  use Ecto.Migration

  @doc """
  Vcs tables for Wave 4 (x0062–x0065).

  vcs_repos    — VcsRepo promoted from DriveNode (x0062)
  vcs_commits  — immutable snapshots; tree_json is Prefab.Payload JSON (x0062)
  vcs_refs     — Branch (mutable head) / Tag (immutable pin) (x0064)
  vcs_certs    — Origin Cert: Ed25519 signature over commit/tag (x0065)

  No FK enforced — application invariant (consistent with project convention).
  """
  def up do
    create table(:vcs_repos, primary_key: false) do
      add :repo_id, :string, primary_key: true, null: false
      add :owner_sub, :string, null: false
      # back-link to drive_nodes.node_id after promote (x0062)
      add :node_id, :string, null: true

      timestamps(type: :utc_datetime_usec)
    end

    create index(:vcs_repos, [:owner_sub])
    create index(:vcs_repos, [:node_id])

    # Immutable snapshots — no updated_at.
    create table(:vcs_commits, primary_key: false) do
      add :commit_id, :string, primary_key: true, null: false
      add :repo_id, :string, null: false
      add :parent_commit_id, :string, null: true
      add :author_sub, :string, null: false
      # JSON matching Assets.Prefab.Payload (schema_version + children[])
      add :tree_json, :text, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_commits, [:repo_id])

    create table(:vcs_refs, primary_key: false) do
      add :ref_id, :string, primary_key: true, null: false
      add :repo_id, :string, null: false
      add :name, :string, null: false
      # branch | tag
      add :kind, :string, null: false
      add :commit_id, :string, null: false
      # is_frozen: true for tags (immutable target), false for branches
      add :is_frozen, :boolean, null: false, default: false

      timestamps(type: :utc_datetime_usec)
    end

    create index(:vcs_refs, [:repo_id])
    # Default Ecto name (`…_index`) — ecto_sqlite3 maps unique violations to this name.
    create unique_index(:vcs_refs, [:repo_id, :name], name: :vcs_refs_repo_id_name_index)

    # Immutable certs — no updated_at.
    create table(:vcs_certs, primary_key: false) do
      add :cert_id, :string, primary_key: true, null: false
      add :subject_sub, :string, null: false
      # commit | tag
      add :target_kind, :string, null: false
      # commit_id or ref_id
      add :target_id, :string, null: false
      # blake3/sha256 of canonical cert message (tamper detection)
      add :target_hash, :string, null: false
      # Base64-encoded Ed25519 signature
      add :signature, :string, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_certs, [:target_id])
    create index(:vcs_certs, [:subject_sub])
  end

  def down do
    drop table(:vcs_certs)
    drop table(:vcs_refs)
    drop table(:vcs_commits)
    drop table(:vcs_repos)
  end
end
