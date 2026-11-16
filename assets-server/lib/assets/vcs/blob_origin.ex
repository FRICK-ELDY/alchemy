defmodule Assets.Vcs.BlobOrigin do
  @moduledoc """
  First-seen blob origin ledger (x00ca / draft §11.5).

  One row per blake3: whoever first brought the blob in via a commit wins.
  Guarantees: insert-once (PK); blake3 normalized to lowercase 64-hex in changeset;
  no update of first_seen fields.
  Non-guarantees: sale-lock rejection; perceptual / near-duplicate detection.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:blake3, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_blob_origins" do
    field :first_commit_hash, :string
    field :author_sub, :string
    field :first_seen_at, :utc_datetime_usec
  end

  def changeset(origin, attrs) do
    origin
    |> cast(attrs, [:blake3, :first_commit_hash, :author_sub, :first_seen_at])
    |> update_change(:blake3, &downcase_hex/1)
    |> validate_required([:blake3, :first_commit_hash, :author_sub, :first_seen_at])
    |> validate_format(:blake3, ~r/^[0-9a-f]{64}$/, message: "must be 64 lowercase hex chars")
  end

  defp downcase_hex(blake3) when is_binary(blake3), do: String.downcase(blake3)
  defp downcase_hex(other), do: other
end
