defmodule World.Repo.Migrations.RoomsX0080HostSession do
  @moduledoc """
  x0080: separate persistent `owner_sub` from compute host (`active_host_sub`) +
  session `state` (`active` | `dormant`) and host lease expiry.

  **Ops note (breaking for live sessions):** existing rows default to `dormant` with
  null `active_host_sub`. After migrate, any in-flight host must re-issue a Host Token
  (or takeover) before the room reappears on Public Sessions.
  """
  use Ecto.Migration

  def up do
    alter table(:rooms) do
      add :active_host_sub, :text, null: true
      add :state, :text, null: false, default: "dormant"
      add :host_lease_expires_at, :utc_datetime, null: true
    end

    create index(:rooms, [:state, :host_lease_expires_at], name: :rooms_state_lease_index)
  end

  def down do
    drop_if_exists index(:rooms, [:state, :host_lease_expires_at], name: :rooms_state_lease_index)

    alter table(:rooms) do
      remove :host_lease_expires_at
      remove :state
      remove :active_host_sub
    end
  end
end
