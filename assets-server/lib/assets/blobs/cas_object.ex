defmodule Assets.Blobs.CasObject do
  @moduledoc """
  Physical CAS metadata (no owner). Content is addressed by blake3.

  `access`:
  - `"authenticated"` (default) — any valid Bearer may GET (room mesh capability)
  - `"grant_required"` — GET requires a BlobGrant for `auth_sub` (x004f)
  """

  use Ecto.Schema
  import Ecto.Changeset

  @access_values ~w(authenticated grant_required)

  @primary_key {:blake3, :string, autogenerate: false}
  @foreign_key_type :string

  schema "cas_objects" do
    field :size, :integer
    field :mime, :string
    field :access, :string, default: "authenticated"

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(object, attrs) do
    object
    |> cast(attrs, [:blake3, :size, :mime, :access])
    |> validate_required([:blake3, :size, :mime, :access])
    |> validate_number(:size, greater_than_or_equal_to: 0)
    |> validate_length(:blake3, is: 64)
    |> validate_inclusion(:access, @access_values)
    |> unique_constraint(:blake3, name: :cas_objects_pkey)
  end
end
