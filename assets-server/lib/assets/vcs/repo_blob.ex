defmodule Assets.Vcs.RepoBlob do
  @moduledoc """
  Repo ↔ blob membership for RepoRead / GC (x00cb / draft §9.2·§15).

  Guarantees: composite PK `(repo_id, blake3)`; blake3 normalized to lowercase
  64-hex in changeset.
  Non-guarantees: push/fork/merge index updates (promote only in x00cb).
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false

  schema "vcs_repo_blobs" do
    field :repo_id, :string, primary_key: true
    field :blake3, :string, primary_key: true
  end

  def changeset(row, attrs) do
    row
    |> cast(attrs, [:repo_id, :blake3])
    |> update_change(:blake3, &downcase_hex/1)
    |> validate_required([:repo_id, :blake3])
    |> validate_format(:blake3, ~r/^[0-9a-f]{64}$/, message: "must be 64 lowercase hex chars")
  end

  defp downcase_hex(blake3) when is_binary(blake3), do: String.downcase(blake3)
  defp downcase_hex(other), do: other
end
