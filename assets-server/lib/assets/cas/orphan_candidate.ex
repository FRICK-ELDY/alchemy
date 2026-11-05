defmodule Assets.CAS.OrphanCandidate do
  @moduledoc """
  CAS blake3 noticed without a CasObject row (x0074).

  Upload compensate never deletes bytes; reaper / compensate / local scan insert
  here so OrphanGc can delete after grace when still unreferenced.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:blake3, :string, autogenerate: false}
  @foreign_key_type :string

  schema "cas_orphan_candidates" do
    field :noticed_at, :utc_datetime_usec
  end

  def changeset(candidate, attrs) do
    candidate
    |> cast(attrs, [:blake3, :noticed_at])
    |> validate_required([:blake3, :noticed_at])
  end
end
