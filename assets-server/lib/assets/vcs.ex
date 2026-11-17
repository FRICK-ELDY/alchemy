defmodule Assets.Vcs do
  @moduledoc """
  Vcs facade — content-addressed objects (x00c9) + Origin Cert v2 (x00ca) +
  visibility / RepoRead index (x00cb / §18.4).

  x0062 / x00c9: promote_prefab/2 — Prefab → VcsRepo + CBOR Tree/Commit + `main`.
  x0064: create_branch/3, create_tag/3, advance_branch/3 — Ref management.
  x00ca: Authorship on promote; Release on create_tag; `vcs_blob_origins` first-seen.
  x00cb: promote defaults `visibility=private` and indexes child meshes in
  `vcs_repo_blobs`; `set_visibility/3` for owner.

  See Assets.ItemRef / Assets.RepoRead for repo@rev read auth (x0063 / x00cb).

  Guarantees:
  - promote: Port encode+verify **outside** DB TX; short TX for ownership/
    grant re-check + objects + commit + repo_commits + `main` + DriveNode +
    Authorship cert + blob origins + repo_blobs.
  - Tag frozen; FF via parent walk on `vcs_commit_parents`.
  - create_tag issues Release cert (subject = repo owner_sub).
  - Cert v2 signs `issued_at_ms` + `kid` (offline-verifiable); signing is TX-outside.
  - Authorship is one row per commit_hash (reuse skips); subject = Commit.author_sub.
  - visibility is private|public; private non-owner reads are :not_found via RepoRead.

  Non-guarantees: push, fork, non-FF merge, sale-lock / :purchased checks,
  multi-key kid lookup / automated rotation, HTTP VCS routes.
  """

  import Ecto.Query, only: [from: 2]

  alias Assets.Blobs
  alias Assets.Drive.Node
  alias Assets.Prefab.Payload
  alias Assets.Repo
  alias Assets.Vcs.BlobOrigin
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.CommitParent
  alias Assets.Vcs.Object
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.Repo, as: VcsRepo
  alias Assets.Vcs.RepoBlob
  alias Assets.Vcs.RepoCommit
  alias Assets.VcsObject

  @max_ff_depth 64

  # ---------------------------------------------------------------------------
  # Promote Prefab → VcsRepo (x00c9)
  # ---------------------------------------------------------------------------

  @doc """
  Promote DriveNode(kind=prefab) to VcsRepo with content-addressed root Commit.

  Creates `main` branch pointing at the new commit_hash.
  New repos start as `visibility=private` and index Prefab child mesh blake3s
  into `vcs_repo_blobs` (RepoRead).
  Port IPC runs before the DB transaction; the TX re-checks ownership, prefab
  kind, CAS blake3 identity, and child grants, then writes only.
  """
  @spec promote_prefab(String.t(), String.t()) ::
          {:ok, %{repo: VcsRepo.t(), commit: Commit.t(), node: Node.t()}}
          | {:error, :not_found | :not_prefab | :no_grant | :invalid_prefab | term()}
  def promote_prefab(owner_sub, node_id) do
    with {:ok, prep} <- prepare_promote(owner_sub, node_id),
         # Sign outside TX unless Authorship already exists for this commit_hash.
         {:ok, authorship} <-
           Cert.prepare_authorship_unless_present(owner_sub, prep.commit_hash) do
      Repo.transaction(fn -> write_promote!(owner_sub, node_id, prep, authorship) end)
    end
  end

  @doc """
  Owner-only visibility change (`private` | `public`).

  public→private hides the repo from others (ItemRef → :not_found).
  Does not detach forks or close PRs (those are later slices).
  private→public does not yet reject `:purchased` blobs (market unshipped).

  Error asymmetry (intentional until HTTP VCS): unknown `repo_id` → `:not_found`;
  known repo but non-owner → `:forbidden`. Read paths (`RepoRead.fetch_readable_repo`,
  ItemRef) collapse private-non-owner to `:not_found`. Align to 404 on HTTP later.
  """
  @spec set_visibility(String.t(), String.t(), String.t()) ::
          {:ok, VcsRepo.t()}
          | {:error, :not_found | :forbidden | :invalid_visibility | term()}
  def set_visibility(repo_id, owner_sub, visibility)
      when is_binary(repo_id) and is_binary(owner_sub) and is_binary(visibility) do
    with {:ok, repo} <- fetch_repo(repo_id),
         :ok <- assert_repo_owner(repo, owner_sub) do
      case VcsRepo.visibility_changeset(repo, %{visibility: visibility}) |> Repo.update() do
        {:ok, updated} -> {:ok, updated}
        {:error, %Ecto.Changeset{}} -> {:error, :invalid_visibility}
      end
    end
  end

  defp assert_repo_owner(%VcsRepo{owner_sub: owner_sub}, owner_sub), do: :ok
  defp assert_repo_owner(%VcsRepo{}, _), do: {:error, :forbidden}

  # ---------------------------------------------------------------------------
  # Branch / Tag
  # ---------------------------------------------------------------------------

  @doc "Create a branch pointing at commit_hash."
  @spec create_branch(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, term()}
  def create_branch(repo_id, name, commit_hash) do
    create_ref(repo_id, name, "branch", commit_hash, false)
  end

  @doc """
  Create an immutable tag pointing at commit_hash and issue a Release cert
  (subject = `VcsRepo.owner_sub`). Signing runs outside the DB TX.
  """
  @spec create_tag(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, term()}
  def create_tag(repo_id, name, commit_hash) do
    with {:ok, %VcsRepo{owner_sub: owner_sub}} <- fetch_repo(repo_id),
         :ok <- assert_commit_in_repo(repo_id, commit_hash),
         {:ok, release} <-
           Cert.prepare_release_unless_present(owner_sub, repo_id, name, commit_hash) do
      Repo.transaction(fn -> write_tag_with_release!(repo_id, name, commit_hash, release) end)
    end
  end

  defp write_tag_with_release!(repo_id, name, commit_hash, release) do
    case create_ref(repo_id, name, "tag", commit_hash, true) do
      {:ok, vref} ->
        Cert.insert_prepared_release!(release)
        vref

      {:error, reason} ->
        Repo.rollback(reason)
    end
  end

  @doc """
  Advance a branch (fast-forward only via parent chain).
  Tags always return `{:error, :frozen}`.
  """
  @spec advance_branch(String.t(), String.t(), String.t()) ::
          {:ok, VcsRef.t()} | {:error, :frozen | :not_fast_forward | :not_found | term()}
  def advance_branch(repo_id, name, new_commit_hash) do
    with {:ok, vref} <- fetch_ref(repo_id, name),
         :ok <- assert_not_frozen(vref),
         :ok <- assert_commit_in_repo(repo_id, new_commit_hash),
         :ok <- assert_fast_forward(vref.commit_hash, new_commit_hash) do
      changeset = VcsRef.advance_changeset(vref, %{commit_hash: new_commit_hash})
      Repo.update(changeset)
    end
  end

  @doc "Resolve a ref name to a VcsRef (branch or tag)."
  @spec get_ref(String.t(), String.t()) :: {:ok, VcsRef.t()} | {:error, :not_found}
  def get_ref(repo_id, name) do
    fetch_ref(repo_id, name)
  end

  # ---------------------------------------------------------------------------
  # Origin Cert v2 (x00ca)
  # ---------------------------------------------------------------------------

  @doc "Issue an Authorship Origin Cert for an existing commit_hash."
  @spec issue_authorship(String.t(), String.t()) ::
          {:ok, Cert.t()} | {:error, term()}
  defdelegate issue_authorship(author_sub, commit_hash), to: Cert

  @doc "Issue a Release Origin Cert for repo_id + tag_name + commit_hash."
  @spec issue_release(String.t(), String.t(), String.t(), String.t()) ::
          {:ok, Cert.t()} | {:error, term()}
  defdelegate issue_release(signer_sub, repo_id, tag_name, commit_hash), to: Cert

  @doc "Verify a stored Origin Cert by cert_id."
  @spec verify_cert(String.t()) :: :ok | {:error, :tampered | :not_found | term()}
  defdelegate verify_cert(cert_id), to: Cert, as: :verify

  # ---------------------------------------------------------------------------
  # Helpers used by ItemRef / Cert
  # ---------------------------------------------------------------------------

  @doc false
  def get_commit(commit_hash) do
    case Repo.get(Commit, commit_hash) do
      nil -> {:error, :not_found}
      %Commit{} = c -> {:ok, c}
    end
  end

  @doc false
  def get_object(hash) do
    case Repo.get(Object, hash) do
      nil -> {:error, :not_found}
      %Object{} = o -> {:ok, o}
    end
  end

  @doc false
  def first_parent_hash(commit_hash) do
    case Repo.get_by(CommitParent, commit_hash: commit_hash, idx: 0) do
      nil -> nil
      %CommitParent{parent_hash: h} -> h
    end
  end

  @doc false
  def commit_in_repo?(repo_id, commit_hash) do
    Repo.exists?(
      from(rc in RepoCommit,
        where: rc.repo_id == ^repo_id and rc.commit_hash == ^commit_hash
      )
    )
  end

  @doc false
  def generate_id_for_test, do: generate_id()

  @doc false
  def insert_commit_for_test!(attrs, parents \\ []) when is_map(attrs) do
    commit_hash = Map.fetch!(attrs, :commit_hash)
    repo_id = Map.get(attrs, :repo_id)
    commit_attrs = Map.drop(attrs, [:repo_id])

    {:ok, commit} = Repo.insert(Commit.changeset(%Commit{}, commit_attrs))

    parents
    |> Enum.with_index()
    |> Enum.each(fn {parent_hash, idx} ->
      {:ok, _} =
        Repo.insert(
          CommitParent.changeset(%CommitParent{}, %{
            commit_hash: commit_hash,
            idx: idx,
            parent_hash: parent_hash
          })
        )
    end)

    if repo_id do
      {:ok, _} =
        Repo.insert(
          RepoCommit.changeset(%RepoCommit{}, %{repo_id: repo_id, commit_hash: commit_hash})
        )
    end

    commit
  end

  @doc false
  def insert_object_for_test!(hash, kind, bytes) do
    case Repo.get(Object, hash) do
      %Object{} = o ->
        o

      nil ->
        {:ok, o} =
          Repo.insert(Object.changeset(%Object{}, %{hash: hash, kind: kind, bytes: bytes}))

        o
    end
  end

  # ---------------------------------------------------------------------------
  # Private — promote prepare (Port IPC, no DB TX)
  # ---------------------------------------------------------------------------

  defp prepare_promote(owner_sub, node_id) do
    with {:ok, node} <- fetch_prefab_for_owner(owner_sub, node_id),
         {:ok, tree_json} <- Assets.CAS.read(node.blake3),
         {:ok, payload} <- decode_payload(tree_json),
         :ok <- Blobs.assert_child_grants(owner_sub, payload.children),
         {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} <- encode_tree(payload),
         created_at_ms = promote_created_at_ms(),
         {:ok, %{bytes: commit_bytes, hash_hex: commit_hash}} <-
           encode_commit(%{
             "tree" => tree_hash,
             "parents" => [],
             "generation" => 1,
             "author_sub" => owner_sub,
             "created_at_ms" => created_at_ms,
             "message" => "promote"
           }),
         :ok <- verify_object(tree_bytes, tree_hash, "tree"),
         :ok <- verify_object(commit_bytes, commit_hash, "commit") do
      {:ok,
       %{
         blake3: node.blake3,
         payload: payload,
         tree_bytes: tree_bytes,
         tree_hash: tree_hash,
         commit_bytes: commit_bytes,
         commit_hash: commit_hash,
         created_at_ms: created_at_ms
       }}
    else
      {:error, :not_found} = err -> err
      {:error, :not_prefab} = err -> err
      {:error, :no_grant} = err -> err
      {:error, :invalid_prefab} = err -> err
      {:error, {:cas_error, reason}} -> {:error, {:cas_error, reason}}
      {:error, reason} -> {:error, reason}
    end
  end

  defp fetch_prefab_for_owner(owner_sub, node_id) do
    case Repo.get_by(Node, node_id: node_id, owner_sub: owner_sub) do
      nil -> {:error, :not_found}
      %Node{kind: "prefab"} = node -> {:ok, node}
      %Node{} -> {:error, :not_prefab}
    end
  end

  defp fetch_prefab_for_owner!(owner_sub, node_id) do
    case fetch_prefab_for_owner(owner_sub, node_id) do
      {:ok, node} -> node
      {:error, reason} -> Repo.rollback(reason)
    end
  end

  defp decode_payload(tree_json) do
    case Payload.from_json(tree_json) do
      {:ok, payload} -> {:ok, payload}
      {:error, _} -> {:error, :invalid_prefab}
    end
  end

  defp assert_child_grants!(owner_sub, %Payload{children: children}) do
    case Blobs.assert_child_grants(owner_sub, children) do
      :ok -> :ok
      {:error, :no_grant} -> Repo.rollback(:no_grant)
    end
  end

  defp encode_tree(%Payload{} = payload) do
    children =
      payload.children
      |> Enum.sort_by(& &1.local_id, :asc)
      |> Enum.map(&child_to_port/1)

    case VcsObject.encode_tree(%{"children" => children}) do
      {:ok, result} -> {:ok, result}
      {:error, reason} -> {:error, {:vcs_object_encode_tree, reason}}
    end
  end

  defp child_to_port(%{local_id: lid, blake3: b3, trs: nil}) do
    %{"local_id" => lid, "blake3" => String.downcase(b3)}
  end

  defp child_to_port(%{local_id: lid, blake3: b3, trs: trs}) do
    %{
      "local_id" => lid,
      "blake3" => String.downcase(b3),
      "trs" => trs_to_b64(trs)
    }
  end

  # Port `encode_tree` re-runs normalize_f32 on decoded TRS. Do not reuse this
  # helper as a source of already-canonical stored bytes without that pass.
  defp trs_to_b64(%{translation: t, rotation: r, scale: s}) do
    floats = t ++ r ++ s

    bin =
      for f <- floats, into: <<>> do
        <<f32_bits(f)::little-float-32>>
      end

    Base.encode64(bin)
  end

  defp f32_bits(v) when is_integer(v), do: v * 1.0
  defp f32_bits(v) when is_float(v), do: v

  defp encode_commit(fields) do
    case VcsObject.encode_commit(fields) do
      {:ok, result} -> {:ok, result}
      {:error, reason} -> {:error, {:vcs_object_encode_commit, reason}}
    end
  end

  defp verify_object(bytes, expect_hash, expect_kind)
       when expect_kind in ["tree", "commit"] do
    case VcsObject.verify(bytes) do
      {:ok, %{"hash_hex" => ^expect_hash, "kind" => ^expect_kind}} ->
        :ok

      {:ok, %{"hash_hex" => ^expect_hash, "kind" => other}} ->
        {:error, {:kind_mismatch, other, expect_kind}}

      {:ok, %{"hash_hex" => ^expect_hash} = summary} ->
        {:error, {:invalid_summary, Map.take(summary, ["kind", "hash_hex"])}}

      {:ok, %{"hash_hex" => other}} when is_binary(other) ->
        {:error, {:hash_mismatch, other, expect_hash}}

      {:ok, summary} ->
        {:error, {:invalid_summary, summary}}

      {:error, reason} ->
        {:error, {:vcs_object_verify, reason}}
    end
  end

  # ---------------------------------------------------------------------------
  # Private — DB writes
  # ---------------------------------------------------------------------------

  defp insert_vcs_repo!(repo_id, owner_sub, node_id) do
    case Repo.insert(
           VcsRepo.changeset(%VcsRepo{}, %{
             repo_id: repo_id,
             owner_sub: owner_sub,
             node_id: node_id,
             visibility: "private"
           })
         ) do
      {:ok, repo} -> repo
      {:error, cs} -> Repo.rollback({:repo_insert_failed, cs})
    end
  end

  defp write_promote!(owner_sub, node_id, prep, authorship) do
    node = fetch_prefab_for_owner!(owner_sub, node_id)
    assert_prefab_unchanged!(node, prep.blake3)
    assert_child_grants!(owner_sub, prep.payload)

    repo_id = generate_id()
    repo = insert_vcs_repo!(repo_id, owner_sub, node_id)

    insert_object!(prep.tree_hash, "tree", prep.tree_bytes)
    insert_object!(prep.commit_hash, "commit", prep.commit_bytes)

    commit =
      insert_commit!(%{
        commit_hash: prep.commit_hash,
        tree_hash: prep.tree_hash,
        generation: 1,
        author_sub: owner_sub,
        created_at_ms: prep.created_at_ms,
        message: "promote"
      })

    insert_repo_commit!(repo_id, prep.commit_hash)
    insert_main_branch!(repo_id, prep.commit_hash)
    # Idempotent: same commit_hash reuses the first Authorship row.
    Cert.insert_prepared_authorship!(authorship)
    insert_blob_origins!(prep.payload.children, prep.commit_hash, owner_sub)
    insert_repo_blobs!(repo_id, prep.payload.children)

    updated_node = update_drive_node_to_vcs!(node, repo_id)
    %{repo: repo, commit: commit, node: updated_node}
  end

  defp insert_repo_blobs!(repo_id, children) when is_list(children) do
    children
    |> Enum.map(fn %{blake3: blake3} -> String.downcase(blake3) end)
    |> Enum.uniq()
    |> Enum.each(fn blake3 ->
      case Repo.insert(
             RepoBlob.changeset(%RepoBlob{}, %{repo_id: repo_id, blake3: blake3}),
             on_conflict: :nothing,
             conflict_target: [:repo_id, :blake3]
           ) do
        {:ok, _} -> :ok
        {:error, cs} -> Repo.rollback({:repo_blob_insert_failed, cs})
      end
    end)
  end

  defp insert_blob_origins!(children, commit_hash, author_sub) when is_list(children) do
    now = DateTime.utc_now() |> DateTime.truncate(:microsecond)

    children
    |> Enum.map(fn %{blake3: blake3} -> String.downcase(blake3) end)
    |> Enum.uniq()
    |> Enum.each(fn blake3 ->
      case Repo.insert(
             BlobOrigin.changeset(%BlobOrigin{}, %{
               blake3: blake3,
               first_commit_hash: commit_hash,
               author_sub: author_sub,
               first_seen_at: now
             }),
             on_conflict: :nothing,
             conflict_target: :blake3
           ) do
        {:ok, _} -> :ok
        {:error, cs} -> Repo.rollback({:blob_origin_insert_failed, cs})
      end
    end)
  end

  defp assert_prefab_unchanged!(%Node{blake3: blake3}, expect_blake3) do
    if blake3 == expect_blake3 do
      :ok
    else
      Repo.rollback(:prefab_changed)
    end
  end

  defp insert_object!(hash, kind, bytes) do
    case Repo.get(Object, hash) do
      %Object{kind: ^kind, bytes: ^bytes} -> :ok
      %Object{} -> Repo.rollback({:object_content_mismatch, hash})
      nil -> insert_object_new!(hash, kind, bytes)
    end
  end

  defp insert_object_new!(hash, kind, bytes) do
    case Repo.insert(
           Object.changeset(%Object{}, %{hash: hash, kind: kind, bytes: bytes}),
           on_conflict: :nothing,
           conflict_target: :hash
         ) do
      {:ok, _} -> assert_object_match!(hash, kind, bytes)
      {:error, cs} -> Repo.rollback({:object_insert_failed, cs})
    end
  end

  defp assert_object_match!(hash, kind, bytes) do
    case Repo.get(Object, hash) do
      %Object{kind: ^kind, bytes: ^bytes} -> :ok
      %Object{} -> Repo.rollback({:object_content_mismatch, hash})
      nil -> Repo.rollback({:object_insert_failed, :missing_after_insert})
    end
  end

  defp insert_commit!(attrs) when is_map(attrs) do
    commit_hash = Map.fetch!(attrs, :commit_hash)

    case Repo.get(Commit, commit_hash) do
      %Commit{} = existing -> assert_commit_match!(existing, attrs)
      nil -> insert_commit_new!(attrs)
    end
  end

  defp insert_commit_new!(attrs) do
    commit_hash = Map.fetch!(attrs, :commit_hash)

    case Repo.insert(Commit.changeset(%Commit{}, attrs),
           on_conflict: :nothing,
           conflict_target: :commit_hash
         ) do
      {:ok, _} ->
        case Repo.get(Commit, commit_hash) do
          %Commit{} = c -> assert_commit_match!(c, attrs)
          nil -> Repo.rollback({:commit_insert_failed, :missing_after_insert})
        end

      {:error, cs} ->
        Repo.rollback({:commit_insert_failed, cs})
    end
  end

  defp assert_commit_match!(%Commit{} = c, attrs) do
    if commit_fields_match?(c, attrs) do
      c
    else
      Repo.rollback({:commit_content_mismatch, c.commit_hash})
    end
  end

  defp commit_fields_match?(%Commit{} = c, attrs) do
    c.tree_hash == Map.fetch!(attrs, :tree_hash) and
      c.generation == Map.fetch!(attrs, :generation) and
      c.author_sub == Map.fetch!(attrs, :author_sub) and
      c.created_at_ms == Map.fetch!(attrs, :created_at_ms) and
      c.message == Map.fetch!(attrs, :message)
  end

  defp insert_repo_commit!(repo_id, commit_hash) do
    case Repo.insert(
           RepoCommit.changeset(%RepoCommit{}, %{repo_id: repo_id, commit_hash: commit_hash})
         ) do
      {:ok, _} -> :ok
      {:error, cs} -> Repo.rollback({:repo_commit_insert_failed, cs})
    end
  end

  defp insert_main_branch!(repo_id, commit_hash) do
    case create_ref(repo_id, "main", "branch", commit_hash, false) do
      {:ok, _} -> :ok
      {:error, reason} -> Repo.rollback({:main_branch_failed, reason})
    end
  end

  defp update_drive_node_to_vcs!(%Node{} = node, repo_id) do
    case Repo.update(Node.changeset(node, %{kind: "vcs_repo", vcs_repo_id: repo_id})) do
      {:ok, updated} -> updated
      {:error, cs} -> Repo.rollback({:node_update_failed, cs})
    end
  end

  defp create_ref(repo_id, name, kind, commit_hash, is_frozen) do
    with :ok <- assert_commit_in_repo(repo_id, commit_hash) do
      Repo.insert(
        VcsRef.changeset(%VcsRef{}, %{
          ref_id: generate_id(),
          repo_id: repo_id,
          name: name,
          kind: kind,
          commit_hash: commit_hash,
          is_frozen: is_frozen
        })
      )
    end
  end

  defp fetch_ref(repo_id, name) do
    case Repo.get_by(VcsRef, repo_id: repo_id, name: name) do
      nil -> {:error, :not_found}
      %VcsRef{} = ref -> {:ok, ref}
    end
  end

  defp fetch_repo(repo_id) do
    case Repo.get(VcsRepo, repo_id) do
      nil -> {:error, :not_found}
      %VcsRepo{} = repo -> {:ok, repo}
    end
  end

  defp assert_not_frozen(%VcsRef{is_frozen: true}), do: {:error, :frozen}
  defp assert_not_frozen(%VcsRef{}), do: :ok

  defp assert_commit_in_repo(repo_id, commit_hash) do
    if commit_in_repo?(repo_id, commit_hash) do
      :ok
    else
      {:error, :not_found}
    end
  end

  defp assert_fast_forward(old_hash, new_hash) do
    do_ff_check(old_hash, new_hash, @max_ff_depth)
  end

  defp do_ff_check(_old, _new, 0), do: {:error, :not_fast_forward}
  defp do_ff_check(old, new, _depth) when old == new, do: :ok

  defp do_ff_check(old, current, depth) do
    case first_parent_hash(current) do
      nil -> {:error, :not_fast_forward}
      parent -> do_ff_check(old, parent, depth - 1)
    end
  end

  defp generate_id do
    :crypto.strong_rand_bytes(16)
    |> Base.encode16(case: :lower)
    |> then(fn hex ->
      <<a::binary-8, b::binary-4, c::binary-4, d::binary-4, e::binary-12>> = hex
      "#{a}-#{b}-#{c}-#{d}-#{e}"
    end)
  end

  # Test-only: `Application.put_env(:assets, :vcs_test_created_at_ms, ms)`.
  defp promote_created_at_ms do
    if mix_test_env?() do
      case Application.get_env(:assets, :vcs_test_created_at_ms) do
        ms when is_integer(ms) and ms >= 0 -> ms
        _ -> System.system_time(:millisecond)
      end
    else
      System.system_time(:millisecond)
    end
  end

  defp mix_test_env? do
    Code.ensure_loaded?(Mix) and function_exported?(Mix, :env, 0) and Mix.env() == :test
  end
end
