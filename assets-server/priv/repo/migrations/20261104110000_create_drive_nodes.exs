defmodule Assets.Repo.Migrations.CreateDriveNodes do
  use Ecto.Migration

  @doc """
  DriveNode: personal Drive folder / blob / prefab hierarchy (x005c + x005d).

  kind values: folder | blob | prefab | vcs_repo
  blake3 references an existing CasObject (blob/prefab payload; grant_required).
  vcs_repo_id is reserved for x0062 Prefab→VcsRepo promotion (nullable).
  No FK enforced — application invariant, consistent with project convention.
  """
  def up do
    create table(:drive_nodes, primary_key: false) do
      add :node_id, :string, primary_key: true, null: false
      add :owner_sub, :string, null: false
      add :parent_id, :string, null: true
      add :kind, :string, null: false
      add :name, :string, null: false
      # blob / prefab payload blake3 (64-char hex); null for folder/vcs_repo-pending
      add :blake3, :string, null: true
      # future VcsRepo link (x0062+); nullable
      add :vcs_repo_id, :string, null: true

      timestamps(type: :utc_datetime_usec)
    end

    create index(:drive_nodes, [:owner_sub])
    create index(:drive_nodes, [:parent_id])
    create index(:drive_nodes, [:owner_sub, :parent_id])

    # Prefab upsert key (x0060). Split indexes: SQLite UNIQUE treats NULL as distinct.
    create unique_index(:drive_nodes, [:owner_sub, :name, :parent_id],
             name: :drive_nodes_prefab_owner_name_parent,
             where: "kind = 'prefab' AND parent_id IS NOT NULL"
           )

    create unique_index(:drive_nodes, [:owner_sub, :name],
             name: :drive_nodes_prefab_owner_name_root,
             where: "kind = 'prefab' AND parent_id IS NULL"
           )
  end

  def down do
    drop table(:drive_nodes)
  end
end
