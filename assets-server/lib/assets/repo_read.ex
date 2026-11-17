defmodule Assets.RepoRead do
  @moduledoc """
  RepoRead — VCS visibility read authorization (x00cb / draft §9.1–9.2).

  Guarantees:
  - `can_read_repo?/2`: owner always; `public` → any authenticated sub.
  - `fetch_readable_repo/2`: missing or private-non-owner → `:not_found` (no existence leak).
  - `allows_blake3?/2`: blake3 is in `vcs_repo_blobs` of at least one readable repo
    (`Repo.exists?/1`; no response cache yet — revisit if GET traffic warrants it).
  - `assert_repo_blobs/2`: every child blake3 is indexed for that repo.
    Call **only after** `can_read` / `fetch_readable_repo` succeeded; this function
    does not re-check visibility. Missing index → `{:error, :incomplete_index}`
    (not `:no_grant` — distinct from personal BlobGrant denial).

  Non-guarantees: unauthenticated public browse; blob GET denial as 404;
  push/fork index maintenance; GET-path caching of RepoRead hits.
  """

  import Ecto.Query, only: [from: 2]

  alias Assets.Repo
  alias Assets.Vcs.Repo, as: VcsRepo
  alias Assets.Vcs.RepoBlob

  @spec can_read_repo?(String.t(), VcsRepo.t()) :: boolean()
  def can_read_repo?(auth_sub, %VcsRepo{} = repo) when is_binary(auth_sub) do
    repo.owner_sub == auth_sub or repo.visibility == "public"
  end

  @spec fetch_readable_repo(String.t(), String.t()) ::
          {:ok, VcsRepo.t()} | {:error, :not_found}
  def fetch_readable_repo(repo_id, auth_sub)
      when is_binary(repo_id) and is_binary(auth_sub) do
    case Repo.get(VcsRepo, repo_id) do
      nil ->
        {:error, :not_found}

      %VcsRepo{} = repo ->
        if can_read_repo?(auth_sub, repo) do
          {:ok, repo}
        else
          {:error, :not_found}
        end
    end
  end

  @spec allows_blake3?(String.t(), String.t()) :: boolean()
  def allows_blake3?(auth_sub, blake3)
      when is_binary(auth_sub) and is_binary(blake3) do
    blake3 = String.downcase(blake3)

    query =
      from rb in RepoBlob,
        join: r in VcsRepo,
        on: r.repo_id == rb.repo_id,
        where: rb.blake3 == ^blake3,
        where: r.owner_sub == ^auth_sub or r.visibility == "public"

    Repo.exists?(query)
  end

  @doc """
  Assert every child blake3 is present in `vcs_repo_blobs` for `repo_id`.

  Caller must already have established read access (`fetch_readable_repo` /
  `can_read_repo?`). This does not check visibility or ownership.
  """
  @spec assert_repo_blobs(String.t(), [%{blake3: String.t()}]) ::
          :ok | {:error, :incomplete_index}
  def assert_repo_blobs(repo_id, children)
      when is_binary(repo_id) and is_list(children) do
    needed =
      children
      |> Enum.map(fn %{blake3: b3} -> String.downcase(b3) end)
      |> Enum.uniq()

    if needed == [] do
      :ok
    else
      present =
        from(rb in RepoBlob,
          where: rb.repo_id == ^repo_id and rb.blake3 in ^needed,
          select: rb.blake3
        )
        |> Repo.all()
        |> MapSet.new()

      if Enum.all?(needed, &MapSet.member?(present, &1)) do
        :ok
      else
        {:error, :incomplete_index}
      end
    end
  end
end
