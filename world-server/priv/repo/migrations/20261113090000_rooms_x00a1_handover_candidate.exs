defmodule World.Repo.Migrations.RoomsX00a1HandoverCandidate do
  @moduledoc """
  x00a1: handover candidate columns for two-stage accept CAS.
  """
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      add :handover_candidate_sub, :text
      add :handover_candidate_expires_at, :utc_datetime
    end
  end

  def down do
    alter table(:rooms) do
      remove :handover_candidate_expires_at
      remove :handover_candidate_sub
    end
  end
end
