defmodule Assets.Vcs.Object do
  @moduledoc """
  VcsObject — content-addressed Tree/Commit bytes (x00c9).

  `hash` is blake3 of `bytes` (canonical CBOR). Kind is `"tree"` | `"commit"`.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:hash, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_objects" do
    field :kind, :string
    field :bytes, :binary

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(object, attrs) do
    object
    |> cast(attrs, [:hash, :kind, :bytes])
    |> validate_required([:hash, :kind, :bytes])
    |> validate_inclusion(:kind, ~w(tree commit))
    |> unique_constraint(:hash, name: :vcs_objects_pkey)
  end
end
