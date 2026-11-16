defmodule Assets.Repo.Migrations.VcsCertsUnique do
  use Ecto.Migration

  @moduledoc """
  x00ca follow-up / idempotent cleanup.

  - Drops the redundant non-unique `(repo_id, tag_name)` index from the first
    x00ca migration revision (if present).
  - Ensures partial unique indexes exist (no-op when already created by the
    consolidated `20261117010000` migration).
  """

  def up do
    drop_if_exists index(:vcs_certs, [:repo_id, :tag_name],
                     name: :vcs_certs_repo_id_tag_name_index
                   )

    # Recreate uniques idempotently for DBs that ran the earlier 10000 without them.
    drop_if_exists index(:vcs_certs, [:target_id], name: :vcs_certs_authorship_commit_index)

    drop_if_exists index(:vcs_certs, [:repo_id, :tag_name],
                     name: :vcs_certs_release_repo_tag_index
                   )

    create unique_index(:vcs_certs, [:target_id],
             name: :vcs_certs_authorship_commit_index,
             where: "cert_kind = 'commit'"
           )

    create unique_index(:vcs_certs, [:repo_id, :tag_name],
             name: :vcs_certs_release_repo_tag_index,
             where: "cert_kind = 'release'"
           )
  end

  def down do
    drop_if_exists index(:vcs_certs, [:repo_id, :tag_name],
                     name: :vcs_certs_release_repo_tag_index
                   )

    drop_if_exists index(:vcs_certs, [:target_id], name: :vcs_certs_authorship_commit_index)

    # Restore the old non-unique lookup index only (pre-consolidation shape).
    create index(:vcs_certs, [:repo_id, :tag_name])
  end
end
