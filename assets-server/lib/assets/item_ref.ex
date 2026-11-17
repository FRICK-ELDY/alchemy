defmodule Assets.ItemRef do
  @moduledoc """
  ItemRef — asset spawn reference (x0055 / x0063 / x00c9 / x00cb).

  Variants:
  - `{:blake3, blake3_hex}` — direct CAS reference
  - `{:repo_rev, repo_id, rev}` — VcsRepo + revision
    rev is a commit_hash (64 hex) OR a VcsRef name (branch/tag).

  Guarantees:
  - resolve/2 for blake3 checks personal BlobGrant.
  - resolve/2 for repo_rev uses RepoRead (`can_read` / private → :not_found)
    and asserts every tree child is in that repo's `vcs_repo_blobs` (x00cb);
    missing index → `:incomplete_index` (not `:no_grant`).
  - Tree children come from `vcs_objects` after Port verify with
    kind=tree and hash_hex == Commit.tree_hash (fail-closed on bytes/PK
    mismatch; not tree_json).

  Non-guarantees: unauthenticated public browse; PROTOCOL_PIN bump;
  vcs_repo take_out / spawn execution.
  """

  alias Assets.Blobs
  alias Assets.Repo
  alias Assets.RepoRead
  alias Assets.Vcs
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Object
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo
  alias Assets.VcsObject

  @type t ::
          {:blake3, String.t()}
          | {:repo_rev, String.t(), String.t()}

  @spec resolve(t(), String.t()) ::
          {:ok, [%{blake3: String.t()}]}
          | {:error,
             :not_found
             | :no_grant
             | :incomplete_index
             | :invalid_rev
             | :invalid_item_ref
             | {:corrupt_tree, term()}
             | term()}
  def resolve({:blake3, blake3}, auth_sub)
      when is_binary(blake3) and byte_size(blake3) == 64 and is_binary(auth_sub) do
    case Blobs.assert_grant(auth_sub, blake3) do
      :ok -> {:ok, [%{blake3: blake3}]}
      {:error, :no_grant} = err -> err
    end
  end

  def resolve({:repo_rev, repo_id, rev}, auth_sub)
      when is_binary(repo_id) and is_binary(rev) and is_binary(auth_sub) do
    with {:ok, repo} <- RepoRead.fetch_readable_repo(repo_id, auth_sub),
         {:ok, commit} <- resolve_rev(repo, rev),
         {:ok, children} <- decode_tree_children(commit),
         :ok <- RepoRead.assert_repo_blobs(repo.repo_id, children) do
      blobs = Enum.map(children, &%{blake3: &1.blake3})
      {:ok, blobs}
    end
  end

  def resolve(_, _), do: {:error, :invalid_item_ref}

  defp decode_tree_children(%Commit{tree_hash: tree_hash}) do
    with {:ok, %Object{bytes: bytes}} <- Vcs.get_object(tree_hash),
         {:ok, summary} <- VcsObject.verify(bytes),
         :ok <- assert_tree_summary(summary, tree_hash) do
      children =
        (summary["child_blake3s_hex"] || [])
        |> Enum.map(&%{blake3: &1, local_id: "", trs: nil})

      {:ok, children}
    else
      {:error, {:corrupt_tree, _} = err} -> {:error, err}
      {:error, reason} -> {:error, {:corrupt_tree, reason}}
    end
  end

  defp assert_tree_summary(%{"kind" => "tree", "hash_hex" => hash}, expect_hash)
       when is_binary(hash) and hash == expect_hash do
    :ok
  end

  defp assert_tree_summary(%{"kind" => kind, "hash_hex" => _hash}, _expect_hash)
       when kind != "tree" do
    {:error, {:corrupt_tree, {:wrong_kind, kind}}}
  end

  defp assert_tree_summary(%{"hash_hex" => hash}, expect_hash)
       when is_binary(hash) and hash != expect_hash do
    {:error, {:corrupt_tree, {:hash_mismatch, hash, expect_hash}}}
  end

  defp assert_tree_summary(summary, _expect_hash) do
    {:error, {:corrupt_tree, {:invalid_summary, summary}}}
  end

  defp resolve_rev(%VcsRepo{repo_id: repo_id}, rev) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: rev) do
      %VcsRef{commit_hash: commit_hash} ->
        fetch_commit_in_repo(repo_id, commit_hash)

      nil ->
        fetch_commit_in_repo(repo_id, rev)
    end
  end

  defp fetch_commit_in_repo(repo_id, commit_hash) do
    if Vcs.commit_in_repo?(repo_id, commit_hash) do
      Vcs.get_commit(commit_hash)
    else
      {:error, :not_found}
    end
  end
end
