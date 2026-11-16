defmodule Assets.Repo.Migrations.VcsContentAddressed do
  use Ecto.Migration

  @moduledoc """
  x00c9: content-addressed VCS objects — **destructive** rebuild.

  Drops UUID/`tree_json` `vcs_commits` + `vcs_refs` and recreates the new shape.
  Pre-x00c9 VCS rows are **not** converted (dev/hermetic only; no shipped prod
  UUID VCS). Origin Cert v1 rows are DELETE'd (UUID signatures invalid).
  `Assets.Vcs.MigrateContentAddressed.run/0` is intentionally a no-op.

  No SQL FK from commits/refs/parents/repo_commits → `vcs_objects`: membership
  and dangling edges are app-enforced (Port verify on read + repo_commits).
  """

  def up do
    # Wipe certs bound to old UUID commit_ids.
    execute("DELETE FROM vcs_certs")

    # Destructive: discard UUID/tree_json era rows (no in-place re-encode).
    drop table(:vcs_refs)
    drop table(:vcs_commits)

    create table(:vcs_objects, primary_key: false) do
      add :hash, :string, primary_key: true, null: false
      add :kind, :string, null: false
      add :bytes, :binary, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_objects, [:kind])

    # tree_hash / commit_hash reference vcs_objects.hash by convention (no FK).
    create table(:vcs_commits, primary_key: false) do
      add :commit_hash, :string, primary_key: true, null: false
      add :tree_hash, :string, null: false
      add :generation, :integer, null: false
      add :author_sub, :string, null: false
      add :created_at_ms, :integer, null: false
      add :message, :string, null: false, default: ""

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_commits, [:tree_hash])
    create index(:vcs_commits, [:author_sub])

    # parent_hash / commit_hash: app-enforced; no FK (allows intentional GC later).
    create table(:vcs_commit_parents, primary_key: false) do
      add :commit_hash, :string, primary_key: true, null: false
      add :idx, :integer, primary_key: true, null: false
      add :parent_hash, :string, null: false
    end

    create index(:vcs_commit_parents, [:parent_hash])

    create table(:vcs_repo_commits, primary_key: false) do
      add :repo_id, :string, primary_key: true, null: false
      add :commit_hash, :string, primary_key: true, null: false
    end

    create index(:vcs_repo_commits, [:commit_hash])

    create table(:vcs_refs, primary_key: false) do
      add :ref_id, :string, primary_key: true, null: false
      add :repo_id, :string, null: false
      add :name, :string, null: false
      add :kind, :string, null: false
      add :commit_hash, :string, null: false
      add :is_frozen, :boolean, null: false, default: false

      timestamps(type: :utc_datetime_usec)
    end

    create index(:vcs_refs, [:repo_id])
    create unique_index(:vcs_refs, [:repo_id, :name], name: :vcs_refs_repo_id_name_index)
    create index(:vcs_refs, [:commit_hash])
  end

  def down do
    drop table(:vcs_refs)
    drop table(:vcs_repo_commits)
    drop table(:vcs_commit_parents)
    drop table(:vcs_commits)
    drop table(:vcs_objects)

    # Restore x0062 shape (empty).
    create table(:vcs_commits, primary_key: false) do
      add :commit_id, :string, primary_key: true, null: false
      add :repo_id, :string, null: false
      add :parent_commit_id, :string, null: true
      add :author_sub, :string, null: false
      add :tree_json, :text, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:vcs_commits, [:repo_id])

    create table(:vcs_refs, primary_key: false) do
      add :ref_id, :string, primary_key: true, null: false
      add :repo_id, :string, null: false
      add :name, :string, null: false
      add :kind, :string, null: false
      add :commit_id, :string, null: false
      add :is_frozen, :boolean, null: false, default: false

      timestamps(type: :utc_datetime_usec)
    end

    create index(:vcs_refs, [:repo_id])
    create unique_index(:vcs_refs, [:repo_id, :name], name: :vcs_refs_repo_id_name_index)
  end
end
