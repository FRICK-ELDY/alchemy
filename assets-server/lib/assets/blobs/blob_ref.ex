defmodule Assets.Blobs.BlobRef do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:blake3, :string, autogenerate: false}
  @foreign_key_type :string

  schema "blob_refs" do
    field :size, :integer
    field :mime, :string
    field :owner_sub, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(blob, attrs) do
    blob
    |> cast(attrs, [:blake3, :size, :mime, :owner_sub])
    |> validate_required([:blake3, :size, :mime, :owner_sub])
    |> validate_number(:size, greater_than_or_equal_to: 0)
    |> validate_length(:blake3, is: 64)
    |> unique_constraint(:blake3, name: :blob_refs_pkey)
  end
end
