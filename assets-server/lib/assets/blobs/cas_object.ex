defmodule Assets.Blobs.CasObject do
  @moduledoc """
  Physical CAS metadata (no owner). Content is addressed by blake3.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:blake3, :string, autogenerate: false}
  @foreign_key_type :string

  schema "cas_objects" do
    field :size, :integer
    field :mime, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(object, attrs) do
    object
    |> cast(attrs, [:blake3, :size, :mime])
    |> validate_required([:blake3, :size, :mime])
    |> validate_number(:size, greater_than_or_equal_to: 0)
    |> validate_length(:blake3, is: 64)
    |> unique_constraint(:blake3, name: :cas_objects_pkey)
  end
end
