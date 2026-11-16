defmodule Assets.Vcs.CommitParent do
  @moduledoc """
  Ordered parent edges for a Commit (x00c9). `idx` 0 = first parent.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false

  schema "vcs_commit_parents" do
    field :commit_hash, :string, primary_key: true
    field :idx, :integer, primary_key: true
    field :parent_hash, :string
  end

  def changeset(row, attrs) do
    row
    |> cast(attrs, [:commit_hash, :idx, :parent_hash])
    |> validate_required([:commit_hash, :idx, :parent_hash])
    |> validate_number(:idx, greater_than_or_equal_to: 0, less_than: 2)
  end
end
