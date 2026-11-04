defmodule Assets.ItemRef do
  @moduledoc """
  ItemRef — asset spawn reference type extending blake3 (x0055 draft) with repo@rev (x0063).

  Variants:
  - `{:blake3, blake3_hex}` — direct CAS reference (x0055 / Wave 0)
  - `{:repo_rev, repo_id, rev}` — VcsRepo + revision (x0063 / Wave 4)
    rev is a commit_id (UUID) OR a VcsRef name (branch/tag).

  Guarantees:
  - resolve/2 checks auth_sub is owner or has BlobGrant for blake3 variant.
  - resolve/2 checks auth_sub is VcsRepo owner for repo_rev variant.
  - Existing blake3-only paths remain unmodified (no regression).
  - Invalid rev → {:error, :not_found}. No-grant → {:error, :no_grant}.

  Non-guarantees: public repo sharing, multi-owner grants, PROTOCOL_PIN bump
  (ItemRef is not on-wire yet; bump required before placing on wire per x0055).
  """

  alias Assets.Blobs.BlobGrant
  alias Assets.Prefab.Payload
  alias Assets.Repo
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo

  import Ecto.Query

  @type t ::
          {:blake3, String.t()}
          | {:repo_rev, String.t(), String.t()}

  @doc """
  Resolve an ItemRef to a list of blake3 refs that auth_sub is permitted to use.

  For :blake3 — checks BlobGrant for auth_sub.
  For :repo_rev — checks VcsRepo ownership, resolves rev to a commit, returns tree children blobs.

  Returns {:ok, [%{blake3: String.t()}]} on success.
  """
  @spec resolve(t(), String.t()) ::
          {:ok, [%{blake3: String.t()}]}
          | {:error, :not_found | :no_grant | :invalid_rev | term()}
  def resolve({:blake3, blake3}, auth_sub)
      when is_binary(blake3) and byte_size(blake3) == 64 and is_binary(auth_sub) do
    case Repo.get_by(BlobGrant, grantee_sub: auth_sub, blake3: blake3) do
      %BlobGrant{} -> {:ok, [%{blake3: blake3}]}
      nil -> {:error, :no_grant}
    end
  end

  def resolve({:repo_rev, repo_id, rev}, auth_sub)
      when is_binary(repo_id) and is_binary(rev) and is_binary(auth_sub) do
    with {:ok, repo} <- fetch_repo_as_owner(repo_id, auth_sub),
         {:ok, commit} <- resolve_rev(repo, rev) do
      case Payload.from_json(commit.tree_json) do
        {:ok, payload} ->
          blobs = Enum.map(payload.children, &%{blake3: &1.blake3})
          {:ok, blobs}

        {:error, reason} ->
          {:error, {:corrupt_tree, reason}}
      end
    end
  end

  def resolve(_, _), do: {:error, :invalid_item_ref}

  # ---------------------------------------------------------------------------
  # Private
  # ---------------------------------------------------------------------------

  defp fetch_repo_as_owner(repo_id, auth_sub) do
    case Repo.get(VcsRepo, repo_id) do
      nil -> {:error, :not_found}
      %VcsRepo{owner_sub: ^auth_sub} = repo -> {:ok, repo}
      %VcsRepo{} -> {:error, :no_grant}
    end
  end

  defp resolve_rev(%VcsRepo{repo_id: repo_id}, rev) do
    # 1. Try as VcsRef name (branch or tag) — commit must still belong to this repo.
    case Repo.get_by(VcsRef, repo_id: repo_id, name: rev) do
      %VcsRef{commit_id: commit_id} ->
        fetch_commit_in_repo(repo_id, commit_id)

      nil ->
        # 2. Try as commit_id directly (UUID format)
        fetch_commit_in_repo(repo_id, rev)
    end
  end

  defp fetch_commit_in_repo(repo_id, commit_id) do
    case Repo.one(from(c in Commit, where: c.commit_id == ^commit_id and c.repo_id == ^repo_id)) do
      nil -> {:error, :not_found}
      %Commit{} = commit -> {:ok, commit}
    end
  end
end
