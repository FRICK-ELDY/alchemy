defmodule Assets.Vcs.Commit do
  @moduledoc """
  VcsCommit — content-addressed Commit summary (x00c9).

  PK `commit_hash` = blake3 of canonical Commit CBOR in `vcs_objects`.
  Parents live in `vcs_commit_parents`. Repo membership in `vcs_repo_commits`.
  """

  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:commit_hash, :string, autogenerate: false}
  @foreign_key_type :string

  schema "vcs_commits" do
    field :tree_hash, :string
    field :generation, :integer
    field :author_sub, :string
    field :created_at_ms, :integer
    field :message, :string

    timestamps(type: :utc_datetime_usec, updated_at: false)
  end

  def changeset(commit, attrs) do
    # Codec allows empty `message` (""). Only `message` is cast with empty
    # preserved; other strings still treat "" as missing via default empty_values.
    commit
    |> cast(attrs, [
      :commit_hash,
      :tree_hash,
      :generation,
      :author_sub,
      :created_at_ms
    ])
    |> cast_message(attrs)
    |> validate_required([
      :commit_hash,
      :tree_hash,
      :generation,
      :author_sub,
      :created_at_ms
    ])
    # `message` may be "" (codec); validate_required treats "" as blank.
    |> validate_message()
    |> validate_number(:generation, greater_than: 0)
    |> unique_constraint(:commit_hash, name: :vcs_commits_pkey)
  end

  defp cast_message(changeset, attrs) when is_map(attrs) do
    case fetch_message(attrs) do
      {:ok, m} when is_binary(m) -> put_change(changeset, :message, m)
      {:ok, _} -> add_error(changeset, :message, "is invalid")
      :error -> changeset
    end
  end

  defp validate_message(changeset) do
    case get_field(changeset, :message) do
      m when is_binary(m) -> changeset
      nil -> add_error(changeset, :message, "can't be blank")
      _ -> add_error(changeset, :message, "is invalid")
    end
  end

  defp fetch_message(attrs) do
    cond do
      Map.has_key?(attrs, :message) -> {:ok, Map.get(attrs, :message)}
      Map.has_key?(attrs, "message") -> {:ok, Map.get(attrs, "message")}
      true -> :error
    end
  end
end
