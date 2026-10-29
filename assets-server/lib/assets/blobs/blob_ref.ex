defmodule Assets.Blobs.BlobRef do
  @moduledoc """
  Per-owner reference to a CasObject. Quota is charged per (owner_sub, blake3).
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false
  @foreign_key_type :string

  schema "blob_refs" do
    field :owner_sub, :string, primary_key: true
    field :blake3, :string, primary_key: true

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(ref, attrs) do
    ref
    |> cast(attrs, [:owner_sub, :blake3])
    |> validate_required([:owner_sub, :blake3])
    |> validate_length(:blake3, is: 64)
    |> unique_constraint([:owner_sub, :blake3], name: :blob_refs_pkey)
  end
end
