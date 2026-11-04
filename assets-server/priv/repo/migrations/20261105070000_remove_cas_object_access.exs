defmodule Assets.Repo.Migrations.RemoveCasObjectAccess do
  use Ecto.Migration

  def up do
    alter table(:cas_objects) do
      remove :access, :string, null: false, default: "authenticated"
    end
  end

  def down do
    alter table(:cas_objects) do
      add :access, :string, null: false, default: "authenticated"
    end
  end
end
