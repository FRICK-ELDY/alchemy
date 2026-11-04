defmodule Assets.Vcs.Commit do
  @moduledoc """
  VcsCommit — immutable snapshot of a Prefab Tree (x0062).

  tree_json is JSON matching Assets.Prefab.Payload (schema_version + children[]).
  Use Assets.Prefab.Payload.from_json/1 to decode it.

  Immutable: no updated_at. Once inserted, a commit is never modified.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:commit_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_commits" do
    field :repo_id, :string
    field :parent_commit_id, :string
    field :author_sub, :string
    field :tree_json, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(commit, attrs) do
    commit
    |> cast(attrs, [:commit_id, :repo_id, :parent_commit_id, :author_sub, :tree_json])
    |> validate_required([:commit_id, :repo_id, :author_sub, :tree_json])
  end
end
