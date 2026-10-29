defmodule Assets.Blobs.BlobGrant do
  @moduledoc """
  Read/hold right for a CasObject (x004f).

  MVP grants `:owned` on PUT finalize. GET uses this only when
  `CasObject.access == "grant_required"`. Default access remains authenticated
  capability (room mesh). Target end-state (option B): all GET gated by grant.
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
