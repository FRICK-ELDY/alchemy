defmodule World.Repo.Migrations.RoomsX009eOverlayEligible do
  @moduledoc """
  x009e: `session_overlay` (temporary scene JSON) + `eligible_host_subs` (takeover allow-list).
  """
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      add :session_overlay, :text, null: false, default: "{\"revision\":0,\"entries\":[]}"
      add :eligible_host_subs, :text, null: false, default: "[]"
    end
  end

  def down do
    alter table(:rooms) do
      remove :eligible_host_subs
      remove :session_overlay
    end
  end
end
