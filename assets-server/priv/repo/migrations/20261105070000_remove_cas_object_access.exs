defmodule Assets.Repo.Migrations.RemoveCasObjectAccess do
  use Ecto.Migration

  def change do
    alter table(:cas_objects) do
      remove :access
    end
  end
end
