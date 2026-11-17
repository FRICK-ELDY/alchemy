defmodule Assets.Vcs.Repo do
  @moduledoc """
  VcsRepo — persistent repository promoted from a Prefab DriveNode (x0062 / x00cb).

  Guarantees: repo_id stable UUID, owner_sub, node_id back-link to drive_nodes;
  visibility is `private` | `public` (promote default `private`).
  Non-guarantees: fork, transfer, default_branch column, branch protection.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:repo_id, :string, autogenerate: false}
  @foreign_key_type :string

  @visibilities ~w(private public)

  schema "vcs_repos" do
    field :owner_sub, :string
    # back-link to drive_nodes.node_id after Prefab→VcsRepo promote (x0062)
    field :node_id, :string
    field :visibility, :string, default: "private"

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(repo, attrs) do
    repo
    |> cast(attrs, [:repo_id, :owner_sub, :node_id, :visibility])
    |> validate_required([:repo_id, :owner_sub, :visibility])
    |> validate_inclusion(:visibility, @visibilities)
  end

  def visibility_changeset(repo, attrs) do
    repo
    |> cast(attrs, [:visibility])
    |> validate_required([:visibility])
    |> validate_inclusion(:visibility, @visibilities)
  end
end
