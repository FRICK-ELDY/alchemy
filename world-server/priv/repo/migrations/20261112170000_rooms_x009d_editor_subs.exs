defmodule World.Repo.Migrations.RoomsX009dEditorSubs do
  @moduledoc """
  x009d Phase 1–2: `editor_subs` JSON array of auth `sub` strings that may mutate
  canonical scene (Builder). Empty means owner-only writers.
  """
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      add :editor_subs, :text, null: false, default: "[]"
    end
  end

  def down do
    alter table(:rooms) do
      remove :editor_subs
    end
  end
end
