defmodule Assets.Quota.QuotaReservation do
  @moduledoc """
  Per-upload quota reservation row (x0074).

  `quota_ledgers.reserved_bytes` remains the aggregate gate; this table tracks
  which blake3 / size / time so TTL reaper can release partial reservations.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id

  schema "quota_reservations" do
    field :owner_sub, :string
    field :blake3, :string
    field :bytes, :integer
    field :reserved_at, :utc_datetime_usec
  end

  def changeset(reservation, attrs) do
    reservation
    |> cast(attrs, [:owner_sub, :blake3, :bytes, :reserved_at])
    |> validate_required([:owner_sub, :blake3, :bytes, :reserved_at])
    |> validate_number(:bytes, greater_than: 0)
    |> unique_constraint([:owner_sub, :blake3])
  end
end
