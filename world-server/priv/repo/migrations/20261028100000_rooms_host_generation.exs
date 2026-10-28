defmodule World.Repo.Migrations.RoomsHostGeneration do
  @moduledoc """
  x003a: persist host_generation + last attested host_pub for replay/handoff gates.
  """
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      add :host_generation, :bigint, null: false, default: 0
      add :attested_host_pub, :text, null: true
    end
  end

  def down do
    alter table(:rooms) do
      remove :attested_host_pub
      remove :host_generation
    end
  end
end
