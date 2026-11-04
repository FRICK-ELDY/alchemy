defmodule Assets.Blobs.BlobGrant do
  @moduledoc """
  Read/hold right for a CasObject (x004f / x006c / x006f).

  PUT finalize issues `:owned` for the uploader. GET is gated by BlobGrant for
  `auth_sub` **or** a covering PlaceRead (option B). Grant presence is the
  capability — there is no `access` column on CasObject or BlobGrant.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @grant_types ~w(owned)

  @primary_key false
  @foreign_key_type :string

  schema "blob_grants" do
    field :grantee_sub, :string, primary_key: true
    field :blake3, :string, primary_key: true
    field :grant_type, :string, default: "owned"

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(grant, attrs) do
    grant
    |> cast(attrs, [:grantee_sub, :blake3, :grant_type])
    |> validate_required([:grantee_sub, :blake3, :grant_type])
    |> validate_length(:blake3, is: 64)
    |> validate_inclusion(:grant_type, @grant_types)
    |> unique_constraint([:grantee_sub, :blake3], name: :blob_grants_pkey)
  end
end
