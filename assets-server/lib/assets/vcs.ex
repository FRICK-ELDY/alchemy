defmodule Assets.Vcs do
  @moduledoc """
  Vcs facade — Wave 4 thin peels (x0062–x0065).

  x0062: promote_prefab/2 — Prefab DriveNode → VcsRepo + initial Commit (same TX).
  x0064: create_branch/3, create_tag/3, advance_branch/3 — minimal Ref management.
  x0065: issue_cert/3, verify_cert/1 — Origin Cert over Commit / Tag.

  See Assets.ItemRef for repo@rev resolution (x0063).

  Guarantees:
  - promote_prefab: same-owner TX; DriveNode kind→vcs_repo + vcs_repo_id set; initial Commit
    tree_json == Prefab payload; existing blake3 path still readable.
  - Tag is immutable (advance_branch refused for frozen refs).
  - Cert: tampered Commit/Tag signature fails verify_cert.
  - Private signing key never logged or returned.

  Non-guarantees: non-FF merge, fork, transfer, room-scoped grants, CRDT.
  """

  alias Assets.Drive.Node
  alias Assets.Repo
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo

  # ---------------------------------------------------------------------------
  # x0062 — Promote Prefab → VcsRepo
  # ---------------------------------------------------------------------------

  @doc """
  Promote a DriveNode(kind=prefab) to a VcsRepo in one transaction (x0062).

  - Creates VcsRepo (owner = node.owner_sub, node_id = node.node_id).
  - Creates initial VcsCommit with tree_json == Prefab payload JSON.
  - Updates DriveNode: kind → "vcs_repo", vcs_repo_id set.
  - DriveNode.blake3 is kept — old blake3 Prefab path remains readable.

  Returns {:ok, %{repo: VcsRepo.t(), commit: Commit.t(), node: Node.t()}}.
  """
  @spec promote_prefab(String.t(), String.t()) ::
          {:ok, %{repo: VcsRepo.t(), commit: Commit.t(), node: Node.t()}}
          | {:error, :not_found | :not_prefab | term()}
  def promote_prefab(owner_sub, node_id) do
    Repo.transaction(fn ->
      node = fetch_prefab_for_owner!(owner_sub, node_id)
      tree_json = load_prefab_json!(node)

      repo_id = generate_id()
      commit_id = generate_id()

      repo =
        insert_vcs_repo!(repo_id, owner_sub, node_id)

      commit =
        insert_vcs_commit!(commit_id, repo_id, owner_sub, tree_json)

      updated_node =
        update_drive_node_to_vcs!(node, repo_id)

      %{repo: repo, commit: commit, node: updated_node}
    end)
  end

  # ---------------------------------------------------------------------------
  # x0064 — Branch / Tag
  # ---------------------------------------------------------------------------

  @doc "Create a branch pointing at commit_id."
  @spec create_branch(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, term()}
  def create_branch(repo_id, name, commit_id) do
    create_ref(repo_id, name, "branch", commit_id, false)
  end

  @doc "Create an immutable tag pointing at commit_id."
  @spec create_tag(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, term()}
  def create_tag(repo_id, name, commit_id) do
    create_ref(repo_id, name, "tag", commit_id, true)
  end

  @doc """
  Advance a branch to a new commit (fast-forward only — new_commit_id must be reachable
  from current head via the parent_commit_id chain).
  Tags are immutable — always returns {:error, :frozen}.
  """
  @spec advance_branch(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, :frozen | :not_fast_forward | :not_found | term()}
  def advance_branch(repo_id, name, new_commit_id) do
    with {:ok, vref} <- fetch_ref(repo_id, name),
         :ok <- assert_not_frozen(vref),
         :ok <- assert_fast_forward(vref.commit_id, new_commit_id) do
      changeset = VcsRef.advance_changeset(vref, %{commit_id: new_commit_id})
      Repo.update(changeset)
    end
  end

  @doc "Resolve a ref name to a VcsRef (branch or tag)."
  @spec get_ref(String.t(), String.t()) :: {:ok, VcsRef.t()} | {:error, :not_found}
  def get_ref(repo_id, name) do
    fetch_ref(repo_id, name)
  end

  # ---------------------------------------------------------------------------
  # x0065 — Origin Cert
  # ---------------------------------------------------------------------------

  @doc "Issue an Origin Cert for a commit or tag (x0065)."
  @spec issue_cert(String.t(), String.t(), String.t()) ::
          {:ok, Cert.t()} | {:error, term()}
  defdelegate issue_cert(subject_sub, target_kind, target_id), to: Cert, as: :issue

  @doc "Verify a stored Origin Cert by cert_id (x0065). Returns :ok or {:error, :tampered}."
  @spec verify_cert(String.t()) :: :ok | {:error, :tampered | :not_found | term()}
  defdelegate verify_cert(cert_id), to: Cert, as: :verify

  # ---------------------------------------------------------------------------
  # Private helpers
  # ---------------------------------------------------------------------------

  defp fetch_prefab_for_owner!(owner_sub, node_id) do
    case Repo.get_by(Node, node_id: node_id, owner_sub: owner_sub) do
      nil -> Repo.rollback(:not_found)
      %Node{kind: "prefab"} = node -> node
      %Node{} -> Repo.rollback(:not_prefab)
    end
  end

  defp load_prefab_json!(%Node{blake3: blake3}) do
    case Assets.CAS.read(blake3) do
      {:ok, json} -> json
      {:error, reason} -> Repo.rollback({:cas_error, reason})
    end
  end

  defp insert_vcs_repo!(repo_id, owner_sub, node_id) do
    changeset =
      VcsRepo.changeset(%VcsRepo{}, %{
        repo_id: repo_id,
        owner_sub: owner_sub,
        node_id: node_id
      })

    case Repo.insert(changeset) do
      {:ok, repo} -> repo
      {:error, cs} -> Repo.rollback({:repo_insert_failed, cs})
    end
  end

  defp insert_vcs_commit!(commit_id, repo_id, author_sub, tree_json) do
    changeset =
      Commit.changeset(%Commit{}, %{
        commit_id: commit_id,
        repo_id: repo_id,
        author_sub: author_sub,
        tree_json: tree_json
      })

    case Repo.insert(changeset) do
      {:ok, commit} -> commit
      {:error, cs} -> Repo.rollback({:commit_insert_failed, cs})
    end
  end

  defp update_drive_node_to_vcs!(%Node{} = node, repo_id) do
    changeset = Node.changeset(node, %{kind: "vcs_repo", vcs_repo_id: repo_id})

    case Repo.update(changeset) do
      {:ok, updated} -> updated
      {:error, cs} -> Repo.rollback({:node_update_failed, cs})
    end
  end

  defp create_ref(repo_id, name, kind, commit_id, is_frozen) do
    changeset =
      VcsRef.changeset(%VcsRef{}, %{
        ref_id: generate_id(),
        repo_id: repo_id,
        name: name,
        kind: kind,
        commit_id: commit_id,
        is_frozen: is_frozen
      })

    Repo.insert(changeset)
  end

  defp fetch_ref(repo_id, name) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: name) do
      nil -> {:error, :not_found}
      %VcsRef{} = ref -> {:ok, ref}
    end
  end

  defp assert_not_frozen(%VcsRef{is_frozen: true}), do: {:error, :frozen}
  defp assert_not_frozen(%VcsRef{}), do: :ok

  @max_ff_depth 64

  # Walk new_commit_id's ancestry to find old_commit_id.
  # If found, the branch advance is a valid fast-forward.
  defp assert_fast_forward(old_commit_id, new_commit_id) do
    do_ff_check(old_commit_id, new_commit_id, @max_ff_depth)
  end

  defp do_ff_check(_old_id, _new_id, 0), do: {:error, :not_fast_forward}

  defp do_ff_check(old_id, new_id, _depth) when old_id == new_id, do: :ok

  defp do_ff_check(old_id, current_id, depth) do
    case Repo.get(Commit, current_id) do
      nil ->
        {:error, :not_fast_forward}

      %Commit{parent_commit_id: nil} ->
        {:error, :not_fast_forward}

      %Commit{parent_commit_id: parent_id} ->
        do_ff_check(old_id, parent_id, depth - 1)
    end
  end

  @doc false
  # Exposed for tests that need to create raw Commit records.
  def generate_id_for_test, do: generate_id()

  defp generate_id do
    :crypto.strong_rand_bytes(16)
    |> Base.encode16(case: :lower)
    |> then(fn hex ->
      <<a::binary-8, b::binary-4, c::binary-4, d::binary-4, e::binary-12>> = hex
      "#{a}-#{b}-#{c}-#{d}-#{e}"
    end)
  end
end
