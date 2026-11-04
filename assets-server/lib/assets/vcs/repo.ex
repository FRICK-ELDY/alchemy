defmodule Assets.Vcs.Repo do
  @moduledoc """
  VcsRepo — persistent repository promoted from a Prefab DriveNode (x0062).

  Guarantees: repo_id stable UUID, owner_sub, node_id back-link to drive_nodes.
  Non-guarantees: fork, transfer, access sharing.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:repo_id, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_repos" do
    field :owner_sub, :string
    # back-link to drive_nodes.node_id after Prefab→VcsRepo promote (x0062)
    field :node_id, :string

    timestamps(type: :utc_datetime_usec)
  end

  def changeset(repo, attrs) do
    repo
    |> cast(attrs, [:repo_id, :owner_sub, :node_id])
    |> validate_required([:repo_id, :owner_sub])
  end
end
