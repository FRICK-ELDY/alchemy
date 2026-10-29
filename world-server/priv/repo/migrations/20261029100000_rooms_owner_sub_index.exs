defmodule World.Repo.Migrations.RoomsOwnerSubIndex do
  @moduledoc """
  Index owner_sub for per-owner room count (x004c).
  """
  use Ecto.Migration

  def change do
    create index(:rooms, [:owner_sub], name: "rooms_owner_sub_index")
  end
end
