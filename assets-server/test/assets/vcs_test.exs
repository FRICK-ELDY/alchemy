defmodule Assets.VcsTest do
  @moduledoc """
  Tests for x0062/x00c9 (promote_prefab content-addressed), x0063 (ItemRef),
  x0064 (Branch/Tag), x0065 / x0096 (Origin Cert v1 over commit_hash).
  """
  use Assets.DataCase, async: false, cas: true

  alias Assets.Drive
  alias Assets.ItemRef
  alias Assets.Prefab.Payload
  alias Assets.Vcs
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.CommitParent
  alias Assets.Vcs.Object
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.VcsObject

  @sub_a "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
  @sub_b "ffffffff-ffff-ffff-ffff-ffffffffffff"

  setup do
    Cert.install_test_key!()
    :ok
  end

  # ---------------------------------------------------------------------------
  # Helpers
  # ---------------------------------------------------------------------------

  defp build_prefab_node(owner_sub, name \\ "test-prefab") do
    {:ok, mesh, _} =
      Assets.Blobs.upload(owner_sub, "vcs-prefab-#{name}", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "c0", blake3: mesh.blake3, trs: nil}
      ])

    {:ok, node} = Drive.save_prefab(owner_sub, name, payload)
    {node, payload}
  end

  defp encode_child_commit!(repo_id, parent_commit, opts \\ []) do
    tree_hash = Keyword.get(opts, :tree_hash, parent_commit.tree_hash)
    parents = Keyword.get(opts, :parents, [parent_commit.commit_hash])
    generation = Keyword.get(opts, :generation, parent_commit.generation + 1)
    message = Keyword.get(opts, :message, "child")
    author = Keyword.get(opts, :author_sub, @sub_a)
    created_at_ms = Keyword.get(opts, :created_at_ms, System.system_time(:millisecond))

    {:ok, %{bytes: bytes, hash_hex: hash}} =
      VcsObject.encode_commit(%{
        "tree" => tree_hash,
        "parents" => parents,
        "generation" => generation,
        "author_sub" => author,
        "created_at_ms" => created_at_ms,
        "message" => message
      })

    Vcs.insert_object_for_test!(hash, "commit", bytes)

    Vcs.insert_commit_for_test!(
      %{
        commit_hash: hash,
        tree_hash: tree_hash,
        generation: generation,
        author_sub: author,
        created_at_ms: created_at_ms,
        message: message,
        repo_id: repo_id
      },
      parents
    )
  end

  defp encode_tree_object!(children) do
    {:ok, %{bytes: bytes, hash_hex: hash}} =
      VcsObject.encode_tree(%{"children" => children})

    Vcs.insert_object_for_test!(hash, "tree", bytes)
    {hash, bytes}
  end

  # ---------------------------------------------------------------------------
  # x0062 / x00c9 — promote_prefab
  # ---------------------------------------------------------------------------

  test "promote_prefab creates VcsRepo + content-addressed Commit + main" do
    {node, _payload} = build_prefab_node(@sub_a)

    assert {:ok, %{repo: repo, commit: commit, node: updated_node}} =
             Vcs.promote_prefab(@sub_a, node.node_id)

    assert repo.owner_sub == @sub_a
    assert repo.node_id == node.node_id
    assert byte_size(commit.commit_hash) == 64
    assert byte_size(commit.tree_hash) == 64
    assert commit.author_sub == @sub_a
    assert commit.message == "promote"
    assert commit.generation == 1

    {:ok, %Object{kind: "commit", bytes: commit_bytes}} = Vcs.get_object(commit.commit_hash)
    assert {:ok, %{"hash_hex" => hash}} = VcsObject.verify(commit_bytes)
    assert hash == commit.commit_hash

    assert {:ok, %VcsRef{kind: "branch", commit_hash: tip}} = Vcs.get_ref(repo.repo_id, "main")
    assert tip == commit.commit_hash

    assert updated_node.kind == "vcs_repo"
    assert updated_node.vcs_repo_id == repo.repo_id
    assert updated_node.blake3 == node.blake3
  end

  test "promote_prefab: Tree object children match Prefab payload (sorted local_id)" do
    {:ok, mesh_a, _} = Assets.Blobs.upload(@sub_a, "vcs-mesh-a", "application/octet-stream")
    {:ok, mesh_b, _} = Assets.Blobs.upload(@sub_a, "vcs-mesh-b", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "mesh_a", blake3: mesh_a.blake3, trs: nil},
        %{
          local_id: "mesh_b",
          blake3: mesh_b.blake3,
          trs: %{
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0]
          }
        }
      ])

    {:ok, node} = Drive.save_prefab(@sub_a, "multi-child", payload)

    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, %Object{bytes: tree_bytes}} = Vcs.get_object(commit.tree_hash)
    assert {:ok, summary} = VcsObject.verify(tree_bytes)
    assert summary["kind"] == "tree"
    assert summary["hash_hex"] == commit.tree_hash

    expected =
      payload.children
      |> Enum.sort_by(& &1.local_id, :asc)
      |> Enum.map(&String.downcase(&1.blake3))

    assert summary["child_blake3s_hex"] == expected
  end

  test "promote_prefab: identical Prefab trees share Tree hash" do
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "shared-mesh", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [%{local_id: "c0", blake3: mesh.blake3, trs: nil}])

    {:ok, n1} = Drive.save_prefab(@sub_a, "same-a", payload)
    {:ok, n2} = Drive.save_prefab(@sub_a, "same-b", payload)

    {:ok, %{commit: c1}} = Vcs.promote_prefab(@sub_a, n1.node_id)
    {:ok, %{commit: c2}} = Vcs.promote_prefab(@sub_a, n2.node_id)

    assert c1.tree_hash == c2.tree_hash
  end

  test "promote_prefab rejects when child grant revoked (x00b4)" do
    {node, _payload} = build_prefab_node(@sub_a, "revoke-promote")

    {:ok, json} = Assets.CAS.read(node.blake3)
    {:ok, %Payload{children: [%{blake3: child_blake3} | _]}} = Payload.from_json(json)

    grant =
      Assets.Repo.get_by!(Assets.Blobs.BlobGrant, grantee_sub: @sub_a, blake3: child_blake3)

    {:ok, _} = Assets.Repo.delete(grant)

    assert {:error, :no_grant} = Vcs.promote_prefab(@sub_a, node.node_id)
    assert Drive.get_node(@sub_a, node.node_id).kind == "prefab"
  end

  test "promote_prefab rejects legacy prefab whose child never had a Grant (x00b4)" do
    fake_child = String.duplicate("e", 64)

    json =
      Jason.encode!(%{
        schema_version: 1,
        children: [%{local_id: "ghost", blake3: fake_child}]
      })

    {:ok, cas, _} = Assets.Blobs.upload(@sub_a, json, "application/json")

    node =
      Assets.Repo.insert!(
        Assets.Drive.Node.changeset(%Assets.Drive.Node{}, %{
          node_id: Ecto.UUID.generate(),
          owner_sub: @sub_a,
          parent_id: nil,
          kind: "prefab",
          name: "legacy-no-child-grant",
          blake3: cas.blake3
        })
      )

    assert {:error, :no_grant} = Vcs.promote_prefab(@sub_a, node.node_id)
    assert Drive.get_node(@sub_a, node.node_id).kind == "prefab"
  end

  test "promote_prefab rejects non-prefab node" do
    {:ok, folder} = Drive.create_folder(@sub_a, "Folder")
    assert {:error, :not_prefab} = Vcs.promote_prefab(@sub_a, folder.node_id)
  end

  test "promote_prefab rejects other user's node" do
    {node, _} = build_prefab_node(@sub_b)
    assert {:error, :not_found} = Vcs.promote_prefab(@sub_a, node.node_id)
  end

  test "promote_prefab: un-promoted Prefab take-out still works" do
    {node2, _} = build_prefab_node(@sub_a, "other-prefab")
    {node1, _} = build_prefab_node(@sub_a, "to-promote")

    {:ok, _} = Vcs.promote_prefab(@sub_a, node1.node_id)

    assert {:ok, %{blobs: [_]}} = Drive.resolve_for_take_out(@sub_a, node2.node_id)
  end

  # ---------------------------------------------------------------------------
  # x0063 — ItemRef repo@rev
  # ---------------------------------------------------------------------------

  test "ItemRef.resolve {:blake3, hash} returns blobs when grant exists" do
    {:ok, cas, _} = Assets.Blobs.upload(@sub_a, "raw", "application/octet-stream")
    assert {:ok, [%{blake3: b3}]} = ItemRef.resolve({:blake3, cas.blake3}, @sub_a)
    assert b3 == cas.blake3
  end

  test "ItemRef.resolve {:blake3, hash} returns no_grant when no BlobGrant" do
    fake = String.duplicate("d", 64)
    assert {:error, :no_grant} = ItemRef.resolve({:blake3, fake}, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, repo_id, commit_hash} resolves tree blobs" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, blobs} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_a)

    assert [%{blake3: _}] = blobs
  end

  test "ItemRef.resolve {:repo_rev, ...} by tag name" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_hash)

    assert {:ok, blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, "v1.0"}, @sub_a)
    assert is_list(blobs)
  end

  test "ItemRef.resolve {:repo_rev, ...} by main branch" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, _blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, "main"}, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, ...} fails for other user (no_grant)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :no_grant} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)
  end

  test "ItemRef.resolve {:repo_rev, ...} fails for unknown rev (:not_found)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, "nonexistent-branch"}, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, ...} rejects revoked child grant (x00b4)" do
    {node, _} = build_prefab_node(@sub_a, "itemref-revoke")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, json} = Assets.CAS.read(node.blake3)
    {:ok, %Payload{children: [%{blake3: child_blake3} | _]}} = Payload.from_json(json)

    grant =
      Assets.Repo.get_by!(Assets.Blobs.BlobGrant, grantee_sub: @sub_a, blake3: child_blake3)

    {:ok, _} = Assets.Repo.delete(grant)

    assert {:error, :no_grant} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, ...} rejects commit tree with never-granted child (x00b4)" do
    {node, _} = build_prefab_node(@sub_a, "itemref-legacy-base")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    fake_child = String.duplicate("f", 64)
    {tree_hash, _} = encode_tree_object!([%{"local_id" => "ghost", "blake3" => fake_child}])
    c2 = encode_child_commit!(repo.repo_id, c1, tree_hash: tree_hash)

    assert {:error, :no_grant} =
             ItemRef.resolve({:repo_rev, repo.repo_id, c2.commit_hash}, @sub_a)
  end

  test "ItemRef.resolve rejects tree bytes that do not hash to tree_hash" do
    {node, _} = build_prefab_node(@sub_a, "itemref-tamper-hash")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    other = String.duplicate("a", 64)

    {:ok, %{bytes: other_bytes}} =
      VcsObject.encode_tree(%{"children" => [%{"local_id" => "x", "blake3" => other}]})

    from(o in Object, where: o.hash == ^commit.tree_hash)
    |> Assets.Repo.update_all(set: [bytes: other_bytes])

    assert {:error, {:corrupt_tree, {:hash_mismatch, _got, expect}}} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_a)

    assert expect == commit.tree_hash
  end

  test "ItemRef.resolve rejects object whose verified kind is not tree" do
    {node, _} = build_prefab_node(@sub_a, "itemref-tamper-kind")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, %Object{bytes: commit_bytes}} = Vcs.get_object(commit.commit_hash)

    from(o in Object, where: o.hash == ^commit.tree_hash)
    |> Assets.Repo.update_all(set: [bytes: commit_bytes])

    assert {:error, {:corrupt_tree, {:wrong_kind, "commit"}}} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_a)
  end

  test "Commit.changeset accepts empty message (codec-compatible)" do
    cs =
      Commit.changeset(%Commit{}, %{
        commit_hash: String.duplicate("1", 64),
        tree_hash: String.duplicate("2", 64),
        generation: 1,
        author_sub: @sub_a,
        created_at_ms: 1,
        message: ""
      })

    assert cs.valid?
    assert Ecto.Changeset.get_field(cs, :message) == ""
  end

  test "Commit.changeset rejects empty commit_hash / tree_hash / author_sub" do
    base = %{
      commit_hash: String.duplicate("1", 64),
      tree_hash: String.duplicate("2", 64),
      generation: 1,
      author_sub: @sub_a,
      created_at_ms: 1,
      message: ""
    }

    refute Commit.changeset(%Commit{}, %{base | commit_hash: ""}).valid?
    refute Commit.changeset(%Commit{}, %{base | tree_hash: ""}).valid?
    refute Commit.changeset(%Commit{}, %{base | author_sub: ""}).valid?
  end

  test "promote_prefab reuses identical Commit row (content-addressed)" do
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "shared-commit-mesh", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [%{local_id: "c0", blake3: mesh.blake3, trs: nil}])

    {:ok, n1} = Drive.save_prefab(@sub_a, "commit-a", payload)
    {:ok, n2} = Drive.save_prefab(@sub_a, "commit-b", payload)

    # Freeze created_at_ms so identical CBOR → identical commit_hash.
    fixed_ms = 1_700_000_000_000
    Application.put_env(:assets, :vcs_test_created_at_ms, fixed_ms)
    on_exit(fn -> Application.delete_env(:assets, :vcs_test_created_at_ms) end)

    assert {:ok, %{commit: c1}} = Vcs.promote_prefab(@sub_a, n1.node_id)
    assert {:ok, %{commit: c2}} = Vcs.promote_prefab(@sub_a, n2.node_id)

    assert c1.commit_hash == c2.commit_hash
    assert c1.tree_hash == c2.tree_hash
  end

  # ---------------------------------------------------------------------------
  # x0064 — Branch / Tag
  # ---------------------------------------------------------------------------

  test "promote creates main; create_branch creates additional branch" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, %VcsRef{kind: "branch", name: "main"}} = Vcs.get_ref(repo.repo_id, "main")

    assert {:ok, %VcsRef{kind: "branch", is_frozen: false}} =
             Vcs.create_branch(repo.repo_id, "dev", commit.commit_hash)
  end

  test "create_tag creates a frozen ref with kind=tag" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, %VcsRef{kind: "tag", is_frozen: true}} =
             Vcs.create_tag(repo.repo_id, "v0.1", commit.commit_hash)
  end

  test "duplicate ref name is rejected" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, _} = Vcs.create_branch(repo.repo_id, "feature", commit.commit_hash)

    assert {:error, %Ecto.Changeset{}} =
             Vcs.create_branch(repo.repo_id, "feature", commit.commit_hash)
  end

  test "create_branch rejects commit from another repo" do
    {node_a, _} = build_prefab_node(@sub_a, "prefab-a")
    {node_b, _} = build_prefab_node(@sub_a, "prefab-b")
    {:ok, %{repo: repo_a}} = Vcs.promote_prefab(@sub_a, node_a.node_id)
    {:ok, %{commit: commit_b}} = Vcs.promote_prefab(@sub_a, node_b.node_id)

    assert {:error, :not_found} =
             Vcs.create_branch(repo_a.repo_id, "dev", commit_b.commit_hash)
  end

  test "advance_branch rejects commit from another repo" do
    {node_a, _} = build_prefab_node(@sub_a, "prefab-a2")
    {node_b, _} = build_prefab_node(@sub_a, "prefab-b2")
    {:ok, %{repo: repo_a}} = Vcs.promote_prefab(@sub_a, node_a.node_id)
    {:ok, %{commit: c_b}} = Vcs.promote_prefab(@sub_a, node_b.node_id)

    assert {:error, :not_found} = Vcs.advance_branch(repo_a.repo_id, "main", c_b.commit_hash)
  end

  test "advance_branch moves branch head to new commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2 = encode_child_commit!(repo.repo_id, c1)

    assert {:ok, %VcsRef{commit_hash: new_hash}} =
             Vcs.advance_branch(repo.repo_id, "main", c2.commit_hash)

    assert new_hash == c2.commit_hash
  end

  test "advance_branch rejects non-fast-forward" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    # Sibling root (no parents) — not a descendant of c1
    c2 =
      encode_child_commit!(repo.repo_id, c1,
        parents: [],
        generation: 1,
        message: "other-root"
      )

    assert {:error, :not_fast_forward} = Vcs.advance_branch(repo.repo_id, "main", c2.commit_hash)
  end

  test "advance_branch on a tag returns :frozen" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_hash)

    assert {:error, :frozen} = Vcs.advance_branch(repo.repo_id, "v1.0", commit.commit_hash)
  end

  test "tag is immutable: cannot be advanced" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "release-1", c1.commit_hash)

    assert {:error, :frozen} = Vcs.advance_branch(repo.repo_id, "release-1", c1.commit_hash)
  end

  test "get_ref returns :not_found for unknown name" do
    assert {:error, :not_found} =
             Vcs.get_ref("00000000-0000-0000-0000-000000000001", "nope")
  end

  # ---------------------------------------------------------------------------
  # x0065 / x0096 / x00c9 — Origin Cert (v1 content-bound)
  # ---------------------------------------------------------------------------

  test "issue_cert creates cert for a commit_hash" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)
    assert cert.subject_sub == @sub_a
    assert cert.target_kind == "commit"
    assert cert.target_id == commit.commit_hash
    assert byte_size(cert.cert_id) > 0
    assert byte_size(cert.signature) > 0
    refute Map.has_key?(cert, :private_key)

    tree_blake3 = Cert.tree_blake3(commit)
    assert tree_blake3 == commit.tree_hash

    message = Cert.canonical_message("commit", commit.commit_hash, @sub_a, tree_blake3, "")
    assert String.starts_with?(message, "v1|")
    assert cert.target_hash == Assets.Blake3.hash_hex(message)
  end

  test "verify_cert returns :ok for a valid cert" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when signature is modified" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: Base.encode64("bad_signature_bytes_padded_to_64_xxxxxxxxxxxx")]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered for corrupt Base64 signature" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: "%%%not-valid-base64%%%"]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when target_hash is altered" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [target_hash: String.duplicate("0", 64)]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when Commit tree_hash changes after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Commit, where: c.commit_hash == ^commit.commit_hash),
      set: [tree_hash: String.duplicate("a", 64)]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when parent row changes after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2 = encode_child_commit!(repo.repo_id, c1)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", c2.commit_hash)

    Assets.Repo.delete_all(from(p in CommitParent, where: p.commit_hash == ^c2.commit_hash))

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when Commit is deleted after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_hash)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_hash == ^commit.commit_hash))

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "canonical_message v1 binds tree object hash and parent commit_hash" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2 = encode_child_commit!(repo.repo_id, c1)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", c2.commit_hash)
    tree_blake3 = Cert.tree_blake3(c2)

    message =
      Cert.canonical_message(
        "commit",
        c2.commit_hash,
        @sub_a,
        tree_blake3,
        c1.commit_hash
      )

    assert message ==
             "v1|commit|#{c2.commit_hash}|#{@sub_a}|#{tree_blake3}|#{c1.commit_hash}"

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :not_found for unknown cert_id" do
    assert {:error, :not_found} = Vcs.verify_cert("00000000-0000-0000-0000-000000000001")
  end

  test "issue_cert for tag kind binds tagged commit tree" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_hash)

    assert {:ok, cert} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)
    assert :ok = Vcs.verify_cert(cert.cert_id)

    tree_blake3 = Cert.tree_blake3(commit)

    message =
      Cert.canonical_message("tag", tag.ref_id, @sub_a, tree_blake3, "")

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
  end

  test "issue_cert returns :not_found for unknown commit" do
    assert {:error, :not_found} =
             Vcs.issue_cert(@sub_a, "commit", String.duplicate("0", 64))
  end

  test "issue_cert returns :not_found for unknown tag" do
    assert {:error, :not_found} =
             Vcs.issue_cert(@sub_a, "tag", "00000000-0000-0000-0000-000000000001")
  end

  test "issue_cert returns :not_found when tag points at missing commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "dangling", commit.commit_hash)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_hash == ^commit.commit_hash))

    assert {:error, :not_found} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)
  end

  test "verify_cert returns :tampered when tag points at missing commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "later-dangling", commit.commit_hash)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_hash == ^commit.commit_hash))

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "issue_cert rejects subject_sub containing pipe" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_cert("evil|sub", "commit", commit.commit_hash)
  end

  test "issue_cert rejects non-binary subject_sub" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_cert(nil, "commit", commit.commit_hash)
  end

  test "canonical_message raises when subject_sub contains pipe" do
    assert_raise ArgumentError, fn ->
      Cert.canonical_message("commit", "id", "evil|sub", String.duplicate("a", 64), "")
    end
  end

  test "issue_cert rejects unknown target_kind" do
    assert {:error, {:invalid_target_kind, "unknown"}} =
             Vcs.issue_cert(@sub_a, "unknown", "some-id")
  end
end
