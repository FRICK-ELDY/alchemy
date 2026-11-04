defmodule Assets.Vcs.Ref do
  @moduledoc """
  VcsRef — Branch (mutable head) or Tag (immutable pin) (x0064).

  Guarantees:
  - Ref name is unique per repo.
  - Tags are frozen (is_frozen=true): updating their commit_id is rejected.
  - Branch heads may be advanced (fast-forward only; x0064 minimal).
  - Ref name must be 1–255 chars, no path separators.

  Non-guarantees: 3-way merge, rebase, reflog.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @kind_values ~w(branch tag)
  @name_max 255

  @primary_key {:ref_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_refs" do
    field :repo_id, :string
    field :name, :string
    field :kind, :string
    field :commit_id, :string
    field :is_frozen, :boolean, default: false

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(ref, attrs) do
    ref
    |> cast(attrs, [:ref_id, :repo_id, :name, :kind, :commit_id, :is_frozen])
    |> validate_required([:ref_id, :repo_id, :name, :kind, :commit_id])
    |> validate_inclusion(:kind, @kind_values)
    |> validate_length(:name, min: 1, max: @name_max)
    |> validate_format(:name, ~r/^[^\x00-\x1f\/\\:*?"<>|]+$/,
      message: "contains invalid characters"
    )
    # ecto_sqlite3 reports this unique violation as `…_index` even when the
    # migration index is named `…_unique` — match the adapter's constraint name.
    |> unique_constraint([:repo_id, :name], name: :vcs_refs_repo_id_name_index)
  end

  @doc "Update-only changeset for advancing a branch (must not be frozen)."
  def advance_changeset(ref, attrs) do
    ref
    |> cast(attrs, [:commit_id])
    |> validate_required([:commit_id])
    |> validate_not_frozen()
  end

  defp validate_not_frozen(changeset) do
    if get_field(changeset, :is_frozen) do
      add_error(changeset, :commit_id, "tag is immutable — target cannot be changed")
    else
      changeset
    end
  end
end
