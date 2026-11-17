defmodule Assets.Repo.Migrations.VcsVisibilityReporead do
  use Ecto.Migration

  @moduledoc """
  x00cb / draft §18.4: `vcs_repos.visibility` + `vcs_repo_blobs` index for RepoRead.

  Existing repos (dev/hermetic) default to `private`.
  `vcs_repo_blobs` is **not** backfilled from pre-x00cb promotes — VCS is
  hermetic / not production-shipped (same premise as x00c9). In-place migrate
  of a local DB with old repos: discard DB or re-promote; otherwise owner
  ItemRef hits `:incomplete_index`. Push/fork will maintain the index later.
  """

  def up do
    alter table(:vcs_repos) do
      add :visibility, :string, null: false, default: "private"
    end

    create table(:vcs_repo_blobs, primary_key: false) do
      add :repo_id, :string, primary_key: true, null: false
      add :blake3, :string, primary_key: true, null: false
    end

    create index(:vcs_repo_blobs, [:blake3])
  end

  def down do
    drop table(:vcs_repo_blobs)

    alter table(:vcs_repos) do
      remove :visibility
    end
  end
end
