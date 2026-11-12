defmodule Assets.VcsTest do
  @moduledoc """
  Tests for x0062 (promote_prefab), x0063 (ItemRef repo@rev), x0064 (Branch/Tag),
  x0065 / x0096 (Origin Cert v1 content-bound).
  """
  use Assets.DataCase, async: false, cas: true

  alias Assets.Drive
  alias Assets.ItemRef
  alias Assets.Prefab.Payload
  alias Assets.Vcs
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Ref, as: VcsRef

  @sub_a "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
  @sub_b "ffffffff-ffff-ffff-ffff-ffffffffffff"
  @blake3 String.duplicate("a", 64)

  setup do
    # Install cert key for x0065 tests
    Cert.install_test_key!()
    :ok
  end

  # ---------------------------------------------------------------------------
  # Helpers
  # ---------------------------------------------------------------------------

  defp build_prefab_node(owner_sub, name \\ "test-prefab") do
    # Upload so child Grant exists — take_out expands children (x007f).
    {:ok, mesh, _} =
      Assets.Blobs.upload(owner_sub, "vcs-prefab-#{name}", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "c0", blake3: mesh.blake3, trs: nil}
      ])

    {:ok, node} = Drive.save_prefab(owner_sub, name, payload)
    {node, payload}
  end

  # ---------------------------------------------------------------------------
  # x0062 — promote_prefab
  # ---------------------------------------------------------------------------

  test "promote_prefab creates VcsRepo + initial Commit" do
    {node, _payload} = build_prefab_node(@sub_a)

    assert {:ok, %{repo: repo, commit: commit, node: updated_node}} =
             Vcs.promote_prefab(@sub_a, node.node_id)

    assert repo.owner_sub == @sub_a
    assert repo.node_id == node.node_id
    assert commit.repo_id == repo.repo_id
    assert commit.author_sub == @sub_a
    assert is_binary(commit.tree_json)

    # DriveNode updated to kind=vcs_repo
    assert updated_node.kind == "vcs_repo"
    assert updated_node.vcs_repo_id == repo.repo_id
    # blake3 preserved for old path
    assert updated_node.blake3 == node.blake3
  end

  test "promote_prefab: Commit tree_json == Prefab payload logical tree" do
    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "mesh_a", blake3: @blake3, trs: nil},
        %{
          local_id: "mesh_b",
          blake3: String.duplicate("b", 64),
          trs: %{
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0]
          }
        }
      ])

    {:ok, node} = Drive.save_prefab(@sub_a, "multi-child", payload)

    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, decoded} = Payload.from_json(commit.tree_json)
    assert length(decoded.children) == 2
    [c0, c1] = decoded.children
    assert c0.local_id == "mesh_a"
    assert c0.blake3 == @blake3
    assert c1.local_id == "mesh_b"
    assert c1.trs.translation == [0.0, 1.0, 0.0]
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
    # A second prefab node not promoted yet
    {node2, _} = build_prefab_node(@sub_a, "other-prefab")
    {node1, _} = build_prefab_node(@sub_a, "to-promote")

    {:ok, _} = Vcs.promote_prefab(@sub_a, node1.node_id)

    # The other node is still kind=prefab and take-out still works
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

  test "ItemRef.resolve {:repo_rev, repo_id, commit_id} resolves tree blobs" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_id}, @sub_a)
    assert [%{blake3: _}] = blobs
  end

  test "ItemRef.resolve {:repo_rev, ...} by tag name" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_id)

    assert {:ok, blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, "v1.0"}, @sub_a)
    assert is_list(blobs)
  end

  test "ItemRef.resolve {:repo_rev, ...} by branch name" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _branch} = Vcs.create_branch(repo.repo_id, "main", commit.commit_id)

    assert {:ok, _blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, "main"}, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, ...} fails for other user (no_grant)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :no_grant} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_id}, @sub_b)
  end

  test "ItemRef.resolve {:repo_rev, ...} fails for unknown rev (:not_found)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, "nonexistent-branch"}, @sub_a)
  end

  # ---------------------------------------------------------------------------
  # x0064 — Branch / Tag
  # ---------------------------------------------------------------------------

  test "create_branch creates a ref with kind=branch" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, %VcsRef{kind: "branch", is_frozen: false}} =
             Vcs.create_branch(repo.repo_id, "main", commit.commit_id)
  end

  test "create_tag creates a frozen ref with kind=tag" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, %VcsRef{kind: "tag", is_frozen: true}} =
             Vcs.create_tag(repo.repo_id, "v0.1", commit.commit_id)
  end

  test "duplicate ref name is rejected" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, _} = Vcs.create_branch(repo.repo_id, "main", commit.commit_id)
    assert {:error, %Ecto.Changeset{}} = Vcs.create_branch(repo.repo_id, "main", commit.commit_id)
  end

  test "create_branch rejects commit from another repo" do
    {node_a, _} = build_prefab_node(@sub_a, "prefab-a")
    {node_b, _} = build_prefab_node(@sub_a, "prefab-b")
    {:ok, %{repo: repo_a}} = Vcs.promote_prefab(@sub_a, node_a.node_id)
    {:ok, %{commit: commit_b}} = Vcs.promote_prefab(@sub_a, node_b.node_id)

    assert {:error, :not_found} =
             Vcs.create_branch(repo_a.repo_id, "main", commit_b.commit_id)
  end

  test "advance_branch rejects commit from another repo" do
    {node_a, _} = build_prefab_node(@sub_a, "prefab-a2")
    {node_b, _} = build_prefab_node(@sub_a, "prefab-b2")
    {:ok, %{repo: repo_a, commit: c_a}} = Vcs.promote_prefab(@sub_a, node_a.node_id)
    {:ok, %{commit: c_b}} = Vcs.promote_prefab(@sub_a, node_b.node_id)
    {:ok, _} = Vcs.create_branch(repo_a.repo_id, "main", c_a.commit_id)

    assert {:error, :not_found} = Vcs.advance_branch(repo_a.repo_id, "main", c_b.commit_id)
  end

  test "advance_branch moves branch head to new commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _branch} = Vcs.create_branch(repo.repo_id, "main", c1.commit_id)

    # Create second commit pointing to c1 as parent
    c2_id = Assets.Vcs.generate_id_for_test()

    c2 =
      Assets.Repo.insert!(
        Commit.changeset(%Commit{}, %{
          commit_id: c2_id,
          repo_id: repo.repo_id,
          parent_commit_id: c1.commit_id,
          author_sub: @sub_a,
          tree_json: c1.tree_json
        })
      )

    assert {:ok, %VcsRef{commit_id: new_cid}} =
             Vcs.advance_branch(repo.repo_id, "main", c2.commit_id)

    assert new_cid == c2.commit_id
  end

  test "advance_branch rejects non-fast-forward" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _} = Vcs.create_branch(repo.repo_id, "main", c1.commit_id)

    # c2 is a new commit without c1 in its ancestry
    c2_id = Assets.Vcs.generate_id_for_test()

    c2 =
      Assets.Repo.insert!(
        Commit.changeset(%Commit{}, %{
          commit_id: c2_id,
          repo_id: repo.repo_id,
          parent_commit_id: nil,
          author_sub: @sub_a,
          tree_json: c1.tree_json
        })
      )

    assert {:error, :not_fast_forward} = Vcs.advance_branch(repo.repo_id, "main", c2.commit_id)
  end

  test "advance_branch on a tag returns :frozen" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_id)

    assert {:error, :frozen} = Vcs.advance_branch(repo.repo_id, "v1.0", commit.commit_id)
  end

  test "tag is immutable: cannot be advanced" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, _tag} = Vcs.create_tag(repo.repo_id, "release-1", c1.commit_id)

    assert {:error, :frozen} = Vcs.advance_branch(repo.repo_id, "release-1", c1.commit_id)
  end

  test "get_ref returns :not_found for unknown name" do
    assert {:error, :not_found} =
             Vcs.get_ref("00000000-0000-0000-0000-000000000001", "nope")
  end

  # ---------------------------------------------------------------------------
  # x0065 / x0096 — Origin Cert (v1 content-bound)
  # ---------------------------------------------------------------------------

  test "issue_cert creates cert for a commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)
    assert cert.subject_sub == @sub_a
    assert cert.target_kind == "commit"
    assert cert.target_id == commit.commit_id
    assert byte_size(cert.cert_id) > 0
    # Signature must be present and non-empty
    assert byte_size(cert.signature) > 0
    # Private key must NOT appear in the cert struct
    refute Map.has_key?(cert, :private_key)

    tree_blake3 = Cert.tree_blake3(commit)
    message = Cert.canonical_message("commit", commit.commit_id, @sub_a, tree_blake3, "")
    assert String.starts_with?(message, "v1|")
    assert cert.target_hash == Assets.Blake3.hash_hex(message)
  end

  test "verify_cert returns :ok for a valid cert" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when signature is modified" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    # Corrupt the signature in-db
    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: Base.encode64("bad_signature_bytes_padded_to_64_xxxxxxxxxxxx")]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered for corrupt Base64 signature" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: "%%%not-valid-base64%%%"]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when target_hash is altered" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    # Corrupt the cert-row message hash anchor (not the Commit body)
    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [target_hash: String.duplicate("0", 64)]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when Commit tree_json changes after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    Assets.Repo.update_all(
      from(c in Commit, where: c.commit_id == ^commit.commit_id),
      set: [tree_json: ~s({"schema_version":1,"children":[]})]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when parent_commit_id changes after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2_id = Vcs.generate_id_for_test()

    {:ok, c2} =
      Assets.Repo.insert(
        Commit.changeset(%Commit{}, %{
          commit_id: c2_id,
          repo_id: repo.repo_id,
          parent_commit_id: c1.commit_id,
          author_sub: @sub_a,
          tree_json: c1.tree_json
        })
      )

    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", c2.commit_id)

    Assets.Repo.update_all(
      from(c in Commit, where: c.commit_id == ^c2.commit_id),
      set: [parent_commit_id: nil]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when Commit is deleted after issue" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", commit.commit_id)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_id == ^commit.commit_id))

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "canonical_message v1 binds tree_blake3 and parent_commit_id" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2_id = Vcs.generate_id_for_test()

    {:ok, c2} =
      Assets.Repo.insert(
        Commit.changeset(%Commit{}, %{
          commit_id: c2_id,
          repo_id: repo.repo_id,
          parent_commit_id: c1.commit_id,
          author_sub: @sub_a,
          tree_json: c1.tree_json
        })
      )

    {:ok, cert} = Vcs.issue_cert(@sub_a, "commit", c2.commit_id)
    tree_blake3 = Cert.tree_blake3(c2)

    message =
      Cert.canonical_message(
        "commit",
        c2.commit_id,
        @sub_a,
        tree_blake3,
        c1.commit_id
      )

    assert message ==
             "v1|commit|#{c2.commit_id}|#{@sub_a}|#{tree_blake3}|#{c1.commit_id}"

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :not_found for unknown cert_id" do
    assert {:error, :not_found} = Vcs.verify_cert("00000000-0000-0000-0000-000000000001")
  end

  test "issue_cert for tag kind binds tagged commit tree" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_id)

    assert {:ok, cert} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)
    assert :ok = Vcs.verify_cert(cert.cert_id)

    tree_blake3 = Cert.tree_blake3(commit)

    message =
      Cert.canonical_message("tag", tag.ref_id, @sub_a, tree_blake3, "")

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
  end

  test "issue_cert returns :not_found for unknown commit" do
    assert {:error, :not_found} =
             Vcs.issue_cert(@sub_a, "commit", "00000000-0000-0000-0000-000000000001")
  end

  test "issue_cert returns :not_found for unknown tag" do
    assert {:error, :not_found} =
             Vcs.issue_cert(@sub_a, "tag", "00000000-0000-0000-0000-000000000001")
  end

  test "issue_cert returns :not_found when tag points at missing commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "dangling", commit.commit_id)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_id == ^commit.commit_id))

    assert {:error, :not_found} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)
  end

  test "verify_cert returns :tampered when tag points at missing commit" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, tag} = Vcs.create_tag(repo.repo_id, "later-dangling", commit.commit_id)
    {:ok, cert} = Vcs.issue_cert(@sub_a, "tag", tag.ref_id)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_id == ^commit.commit_id))

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "issue_cert rejects subject_sub containing pipe" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_cert("evil|sub", "commit", commit.commit_id)
  end

  test "issue_cert rejects non-binary subject_sub" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_cert(nil, "commit", commit.commit_id)
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
