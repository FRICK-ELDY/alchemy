defmodule World.Repo.Migrations.RoomsRoomKeyRotatedAt do
  @moduledoc """
  x007e: wall-clock of last Room key rotate (periodic + narrow + owner rotate).
  Backfill to `inserted_at` so existing rooms are not mass-rotated on deploy.
  """
  use Ecto.Migration

  def change do
    alter table(:rooms) do
      add :room_key_rotated_at, :utc_datetime, null: true
    end

    execute(
      "UPDATE rooms SET room_key_rotated_at = inserted_at WHERE room_key_rotated_at IS NULL",
      ""
    )
  end
end
