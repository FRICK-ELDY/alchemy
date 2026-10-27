defmodule World.Repo.Migrations.RoomsVisibility do
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      # Demo-compatible default: public (authenticated guests keep working).
      add :visibility, :text, null: false, default: "public"
    end
  end

  def down do
    alter table(:rooms) do
      remove :visibility
    end
  end
end
