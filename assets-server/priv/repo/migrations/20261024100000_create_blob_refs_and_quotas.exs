defmodule Assets.Repo.Migrations.CreateBlobRefsAndQuotas do
  use Ecto.Migration

  def change do
    create table(:blob_refs, primary_key: false) do
      add :blake3, :string, primary_key: true, null: false
      add :size, :integer, null: false
      add :mime, :string, null: false, default: "application/octet-stream"
      add :owner_sub, :string, null: false

      timestamps(type: :utc_datetime_usec, updated_at: false)
    end

    create index(:blob_refs, [:owner_sub])

    create table(:quota_ledgers, primary_key: false) do
      add :owner_sub, :string, primary_key: true, null: false
      add :used_bytes, :integer, null: false, default: 0
      add :limit_bytes, :integer, null: false

      timestamps(type: :utc_datetime_usec)
    end
  end
end
