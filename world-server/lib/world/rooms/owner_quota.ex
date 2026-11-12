defmodule World.Rooms.OwnerQuota do
  @moduledoc """
  Per-owner room count ledger (x0097).

  Guarantees: `room_count` is bumped only via conditional UPDATE in the same
  TX as `rooms` insert (`World.Rooms.create/3`).
  Non-guarantees: Room DELETE decrement (no DELETE API yet); healing of
  pre-x0097 overshoot beyond backfill.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:owner_sub, :string, autogenerate: false}
  @foreign_key_type :string

  schema "room_owner_quotas" do
    field :room_count, :integer, default: 0

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(quota, attrs) do
    quota
    |> cast(attrs, [:owner_sub, :room_count])
    |> validate_required([:owner_sub, :room_count])
    |> validate_number(:room_count, greater_than_or_equal_to: 0)
  end
end
