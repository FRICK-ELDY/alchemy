defmodule Assets.Vcs.RepoCommit do
  @moduledoc """
  Repo ↔ Commit membership (x00c9). Objects are global; membership is per-repo.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false

  schema "vcs_repo_commits" do
    field :repo_id, :string, primary_key: true
    field :commit_hash, :string, primary_key: true
  end

  def changeset(row, attrs) do
    row
    |> cast(attrs, [:repo_id, :commit_hash])
    |> validate_required([:repo_id, :commit_hash])
  end
end
