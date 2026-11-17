defmodule Assets.VcsTest do
  @moduledoc """
  Tests for x0062/x00c9 (promote_prefab content-addressed), x0063 (ItemRef),
  x0064 (Branch/Tag), x00ca (Origin Cert v2 + blob origins),
  x00cb (visibility / RepoRead / private 404),
  x00cc (negotiate / push / branch protection).
  """
  use Assets.DataCase, async: false, cas: true

  import Ecto.Query, only: [from: 2]

  alias Assets.Drive
  alias Assets.ItemRef
  alias Assets.Prefab.Payload
  alias Assets.RepoRead
  alias Assets.Vcs
  alias Assets.Vcs.BlobOrigin
  alias Assets.Vcs.Cert
  alias Assets.Vcs.Commit
  alias Assets.Vcs.Object
  alias Assets.Vcs.Ref, as: VcsRef
  alias Assets.Vcs.RepoBlob
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
    assert repo.visibility == "private"
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

  test "ItemRef.resolve {:repo_rev, ...} private other user is :not_found (x00cb)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)
  end

  test "ItemRef.resolve {:repo_rev, ...} fails for unknown rev (:not_found)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, "nonexistent-branch"}, @sub_a)
  end

  test "ItemRef.resolve + Blobs.fetch succeed via RepoRead after grant revoke (x00cb)" do
    {node, _} = build_prefab_node(@sub_a, "itemref-revoke")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, json} = Assets.CAS.read(node.blake3)
    {:ok, %Payload{children: [%{blake3: child_blake3} | _]}} = Payload.from_json(json)

    grant =
      Assets.Repo.get_by!(Assets.Blobs.BlobGrant, grantee_sub: @sub_a, blake3: child_blake3)

    {:ok, _} = Assets.Repo.delete(grant)

    assert {:ok, [%{blake3: ^child_blake3}]} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_a)

    assert RepoRead.allows_blake3?(@sub_a, child_blake3)
    assert {:ok, _bytes, _obj} = Assets.Blobs.fetch(child_blake3, @sub_a)
  end

  test "ItemRef.resolve {:repo_rev, ...} rejects tree child missing from vcs_repo_blobs (x00cb)" do
    {node, _} = build_prefab_node(@sub_a, "itemref-legacy-base")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    fake_child = String.duplicate("f", 64)
    {tree_hash, _} = encode_tree_object!([%{"local_id" => "ghost", "blake3" => fake_child}])
    c2 = encode_child_commit!(repo.repo_id, c1, tree_hash: tree_hash)

    assert {:error, :incomplete_index} =
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

  # ---------------------------------------------------------------------------
  # x00cb — visibility / RepoRead / private 404
  # ---------------------------------------------------------------------------

  test "promote_prefab indexes child blake3s into vcs_repo_blobs (x00cb)" do
    {node, payload} = build_prefab_node(@sub_a, "repo-blobs")
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)

    [%{blake3: child}] = payload.children
    child = String.downcase(child)

    assert %RepoBlob{blake3: ^child} =
             Assets.Repo.get_by!(RepoBlob, repo_id: repo.repo_id, blake3: child)
  end

  test "set_visibility owner can flip private↔public; non-owner forbidden (x00cb)" do
    {node, _} = build_prefab_node(@sub_a, "vis-flip")
    {:ok, %{repo: repo}} = Vcs.promote_prefab(@sub_a, node.node_id)
    assert repo.visibility == "private"

    assert {:ok, %{visibility: "public"}} =
             Vcs.set_visibility(repo.repo_id, @sub_a, "public")

    assert {:error, :forbidden} =
             Vcs.set_visibility(repo.repo_id, @sub_b, "private")

    assert {:ok, %{visibility: "private"}} =
             Vcs.set_visibility(repo.repo_id, @sub_a, "private")

    assert {:error, :invalid_visibility} =
             Vcs.set_visibility(repo.repo_id, @sub_a, "friends")
  end

  test "public repo: other user ItemRef resolve + RepoRead GET (x00cb)" do
    {node, payload} = build_prefab_node(@sub_a, "public-read")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    [%{blake3: child}] = payload.children

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)

    assert {:error, :forbidden} = Assets.Blobs.fetch(child, @sub_b)

    assert {:ok, _} = Vcs.set_visibility(repo.repo_id, @sub_a, "public")

    assert {:ok, [%{blake3: ^child}]} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)

    assert RepoRead.allows_blake3?(@sub_b, child)
    assert {:ok, _bytes, _obj} = Assets.Blobs.fetch(child, @sub_b)
  end

  test "public→private hides ItemRef and RepoRead for others (x00cb)" do
    {node, payload} = build_prefab_node(@sub_a, "reprivate")
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    [%{blake3: child}] = payload.children

    assert {:ok, _} = Vcs.set_visibility(repo.repo_id, @sub_a, "public")
    assert {:ok, _} = ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)

    assert {:ok, _} = Vcs.set_visibility(repo.repo_id, @sub_a, "private")

    assert {:error, :not_found} =
             ItemRef.resolve({:repo_rev, repo.repo_id, commit.commit_hash}, @sub_b)

    refute RepoRead.allows_blake3?(@sub_b, child)
    assert {:error, :forbidden} = Assets.Blobs.fetch(child, @sub_b)
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
  # x00ca — Origin Cert v2 + blob origins
  # ---------------------------------------------------------------------------

  defp authorship_cert!(commit_hash) do
    case Cert.get_authorship(commit_hash) do
      %Cert{} = cert -> cert
      nil -> flunk("expected Authorship cert for #{commit_hash}")
    end
  end

  defp release_cert!(repo_id, tag_name) do
    case Cert.get_release(repo_id, tag_name) do
      %Cert{} = cert -> cert
      nil -> flunk("expected Release cert for #{repo_id}/#{tag_name}")
    end
  end

  defp authorship_count(commit_hash) do
    Assets.Repo.aggregate(
      from(c in Cert, where: c.cert_kind == "commit" and c.target_id == ^commit_hash),
      :count
    )
  end

  test "promote_prefab issues Authorship cert v2 with issued_at_ms and kid" do
    Application.put_env(:assets, :vcs_test_issued_at_ms, 1_700_000_000_000)
    {node, _} = build_prefab_node(@sub_a)

    assert {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    cert = authorship_cert!(commit.commit_hash)
    assert cert.subject_sub == @sub_a
    assert cert.cert_kind == "commit"
    assert cert.target_id == commit.commit_hash
    assert cert.issued_at_ms == 1_700_000_000_000
    assert cert.kid == "default"
    assert cert.message_version == "v2"
    refute Map.has_key?(cert, :private_key)

    message =
      Cert.authorship_message(commit.commit_hash, @sub_a, cert.issued_at_ms, cert.kid)

    assert message ==
             "v2|commit|#{commit.commit_hash}|#{@sub_a}|#{cert.issued_at_ms}|#{cert.kid}"

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
    assert :ok = Vcs.verify_cert(cert.cert_id)

    assert :ok =
             Cert.verify_offline_authorship(
               commit.commit_hash,
               @sub_a,
               cert.issued_at_ms,
               cert.kid,
               cert.signature
             )
  after
    Application.delete_env(:assets, :vcs_test_issued_at_ms)
  end

  test "promote_prefab records blob origins for child meshes" do
    {node, payload} = build_prefab_node(@sub_a)
    child_blake3 = hd(payload.children).blake3 |> String.downcase()

    assert {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    origin = Assets.Repo.get!(BlobOrigin, child_blake3)
    assert origin.first_commit_hash == commit.commit_hash
    assert origin.author_sub == @sub_a
    assert %DateTime{} = origin.first_seen_at
  end

  test "blob origins keep first author on later promote of same blake3" do
    {node_a, payload} = build_prefab_node(@sub_a, "origin-a")
    child_blake3 = hd(payload.children).blake3 |> String.downcase()
    {:ok, %{commit: c1}} = Vcs.promote_prefab(@sub_a, node_a.node_id)

    # Re-upload same bytes under another owner → same blake3, new prefab node.
    {:ok, mesh_bytes} = Assets.CAS.read(child_blake3)

    assert {:ok, mesh_b, _} =
             Assets.Blobs.upload(@sub_b, mesh_bytes, "application/octet-stream")

    assert mesh_b.blake3 == child_blake3

    {:ok, payload_b} =
      Payload.new(1, [%{local_id: "c0", blake3: child_blake3, trs: nil}])

    {:ok, node_b} = Drive.save_prefab(@sub_b, "origin-b", payload_b)
    assert {:ok, %{commit: c2}} = Vcs.promote_prefab(@sub_b, node_b.node_id)
    assert c2.commit_hash != c1.commit_hash

    origin = Assets.Repo.get!(BlobOrigin, child_blake3)
    assert origin.author_sub == @sub_a
    assert origin.first_commit_hash == c1.commit_hash
  end

  test "create_tag issues Release cert for repo owner" do
    Application.put_env(:assets, :vcs_test_issued_at_ms, 1_700_000_000_100)
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:ok, _tag} = Vcs.create_tag(repo.repo_id, "v1.0", commit.commit_hash)

    cert = release_cert!(repo.repo_id, "v1.0")
    assert cert.subject_sub == @sub_a
    assert cert.cert_kind == "release"
    assert cert.target_id == commit.commit_hash
    assert cert.repo_id == repo.repo_id
    assert cert.tag_name == "v1.0"
    assert cert.issued_at_ms == 1_700_000_000_100
    assert cert.kid == "default"

    message =
      Cert.release_message(
        repo.repo_id,
        "v1.0",
        commit.commit_hash,
        @sub_a,
        cert.issued_at_ms,
        cert.kid
      )

    assert cert.target_hash == Assets.Blake3.hash_hex(message)
    assert :ok = Vcs.verify_cert(cert.cert_id)

    assert :ok =
             Cert.verify_offline_release(
               repo.repo_id,
               "v1.0",
               commit.commit_hash,
               @sub_a,
               cert.issued_at_ms,
               cert.kid,
               cert.signature
             )
  after
    Application.delete_env(:assets, :vcs_test_issued_at_ms)
  end

  test "verify_cert returns :ok for promote Authorship" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)
    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when signature is modified" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: Base.encode64("bad_signature_bytes_padded_to_64_xxxxxxxxxxxx")]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered for corrupt Base64 signature" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [signature: "%%%not-valid-base64%%%"]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when target_hash is altered" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [target_hash: String.duplicate("0", 64)]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "verify_cert returns :tampered when issued_at_ms is altered" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)

    Assets.Repo.update_all(
      from(c in Cert, where: c.cert_id == ^cert.cert_id),
      set: [issued_at_ms: cert.issued_at_ms + 1]
    )

    assert {:error, :tampered} = Vcs.verify_cert(cert.cert_id)
  end

  test "Authorship remains valid after Commit row is deleted (offline fields)" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)

    Assets.Repo.delete_all(from(c in Commit, where: c.commit_hash == ^commit.commit_hash))

    assert :ok = Vcs.verify_cert(cert.cert_id)

    assert :ok =
             Cert.verify_offline_authorship(
               commit.commit_hash,
               @sub_a,
               cert.issued_at_ms,
               cert.kid,
               cert.signature
             )
  end

  test "issue_authorship for child commit binds v2 message" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    c2 = encode_child_commit!(repo.repo_id, c1)
    assert {:ok, cert} = Vcs.issue_authorship(@sub_a, c2.commit_hash)

    message =
      Cert.authorship_message(c2.commit_hash, @sub_a, cert.issued_at_ms, cert.kid)

    assert String.starts_with?(message, "v2|commit|")
    assert cert.target_hash == Assets.Blake3.hash_hex(message)
    assert :ok = Vcs.verify_cert(cert.cert_id)
  end

  test "re-promote of same commit_hash keeps a single Authorship row" do
    Application.put_env(:assets, :vcs_test_created_at_ms, 1_700_000_000_200)
    Application.put_env(:assets, :vcs_test_issued_at_ms, 1_700_000_000_201)

    {node1, payload} = build_prefab_node(@sub_a, "same-a")
    assert {:ok, %{commit: c1}} = Vcs.promote_prefab(@sub_a, node1.node_id)
    first = authorship_cert!(c1.commit_hash)
    assert authorship_count(c1.commit_hash) == 1

    # Same child blake3 + author + created_at_ms → same commit_hash.
    child_blake3 = hd(payload.children).blake3

    {:ok, payload2} =
      Payload.new(1, [%{local_id: "c0", blake3: child_blake3, trs: nil}])

    {:ok, node2} = Drive.save_prefab(@sub_a, "same-b", payload2)
    assert {:ok, %{commit: c2}} = Vcs.promote_prefab(@sub_a, node2.node_id)
    assert c2.commit_hash == c1.commit_hash

    assert authorship_count(c1.commit_hash) == 1
    second = authorship_cert!(c1.commit_hash)
    assert second.cert_id == first.cert_id
    assert second.issued_at_ms == first.issued_at_ms
  after
    Application.delete_env(:assets, :vcs_test_created_at_ms)
    Application.delete_env(:assets, :vcs_test_issued_at_ms)
  end

  test "issue_authorship is idempotent and rejects foreign author_sub" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    first = authorship_cert!(commit.commit_hash)

    assert {:ok, again} = Vcs.issue_authorship(@sub_a, commit.commit_hash)
    assert again.cert_id == first.cert_id
    assert authorship_count(commit.commit_hash) == 1

    assert {:error, :author_mismatch} =
             Vcs.issue_authorship(@sub_b, commit.commit_hash)
  end

  test "prepare_authorship_unless_present returns :cert_conflict on subject mismatch" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    _ = authorship_cert!(commit.commit_hash)

    # Existing row subject is @sub_a; asking with a different subject is a conflict
    # (distinct from public issue_* which fails earlier with :author_mismatch).
    assert {:error, :cert_conflict} =
             Cert.prepare_authorship_unless_present(@sub_b, commit.commit_hash)
  end

  test "issue_release rejects non-owner and missing tag" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :not_found} =
             Vcs.issue_release(@sub_a, repo.repo_id, "missing", commit.commit_hash)

    assert {:ok, _} = Vcs.create_tag(repo.repo_id, "v2.0", commit.commit_hash)
    first = release_cert!(repo.repo_id, "v2.0")

    assert {:ok, again} =
             Vcs.issue_release(@sub_a, repo.repo_id, "v2.0", commit.commit_hash)

    assert again.cert_id == first.cert_id

    assert {:error, :owner_mismatch} =
             Vcs.issue_release(@sub_b, repo.repo_id, "v2.0", commit.commit_hash)

    c2 = encode_child_commit!(repo.repo_id, commit)

    assert {:error, :commit_mismatch} =
             Vcs.issue_release(@sub_a, repo.repo_id, "v2.0", c2.commit_hash)
  end

  test "prepare_release_unless_present returns :cert_conflict on subject mismatch" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{repo: repo, commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    assert {:ok, _} = Vcs.create_tag(repo.repo_id, "v3.0", commit.commit_hash)
    _ = release_cert!(repo.repo_id, "v3.0")

    assert {:error, :cert_conflict} =
             Cert.prepare_release_unless_present(
               @sub_b,
               repo.repo_id,
               "v3.0",
               commit.commit_hash
             )
  end

  test "BlobOrigin changeset downcases blake3 hex" do
    {node, payload} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    lower = hd(payload.children).blake3 |> String.downcase()
    upper = String.upcase(lower)

    # Direct insert with uppercase collapses to the same PK as promote's origin.
    cs =
      BlobOrigin.changeset(%BlobOrigin{}, %{
        blake3: upper,
        first_commit_hash: commit.commit_hash,
        author_sub: @sub_b,
        first_seen_at: DateTime.utc_now() |> DateTime.truncate(:microsecond)
      })

    assert Ecto.Changeset.get_change(cs, :blake3) == lower

    assert {:ok, _} =
             Assets.Repo.insert(cs, on_conflict: :nothing, conflict_target: :blake3)

    origin = Assets.Repo.get!(BlobOrigin, lower)
    assert origin.blake3 == lower
    assert origin.author_sub == @sub_a
  end

  test "verify_offline_authorship accepts explicit public key" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)
    cert = authorship_cert!(commit.commit_hash)
    pub = Application.get_env(:assets, :cert_signing_pub_key)

    assert :ok =
             Cert.verify_offline_authorship(
               commit.commit_hash,
               @sub_a,
               cert.issued_at_ms,
               cert.kid,
               cert.signature,
               pub
             )

    assert {:error, :tampered} =
             Cert.verify_offline_authorship(
               commit.commit_hash,
               @sub_a,
               cert.issued_at_ms,
               cert.kid,
               cert.signature,
               Base.encode64(:crypto.strong_rand_bytes(32))
             )
  end

  test "verify_cert returns :not_found for unknown cert_id" do
    assert {:error, :not_found} = Vcs.verify_cert("00000000-0000-0000-0000-000000000001")
  end

  test "issue_authorship returns :not_found for unknown commit" do
    assert {:error, :not_found} =
             Vcs.issue_authorship(@sub_a, String.duplicate("0", 64))
  end

  test "issue_release returns :not_found for unknown commit" do
    assert {:error, :not_found} =
             Vcs.issue_release(@sub_a, "repo", "v1", String.duplicate("0", 64))
  end

  test "issue_authorship rejects subject_sub containing pipe" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_authorship("evil|sub", commit.commit_hash)
  end

  test "issue_authorship rejects non-binary subject_sub" do
    {node, _} = build_prefab_node(@sub_a)
    {:ok, %{commit: commit}} = Vcs.promote_prefab(@sub_a, node.node_id)

    assert {:error, :invalid_subject_sub} =
             Vcs.issue_authorship(nil, commit.commit_hash)
  end

  test "authorship_message raises when subject_sub contains pipe" do
    assert_raise ArgumentError, fn ->
      Cert.authorship_message(String.duplicate("a", 64), "evil|sub", 0, "default")
    end
  end

  # ---------------------------------------------------------------------------
  # x00cc — negotiate / push / branch protection
  # ---------------------------------------------------------------------------

  defp encode_push_child(parent_commit, mesh_blake3, opts \\ []) do
    message = Keyword.get(opts, :message, "push-child")
    author = Keyword.get(opts, :author_sub, @sub_a)
    created_at_ms = Keyword.get(opts, :created_at_ms, System.system_time(:millisecond))

    {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "c0", "blake3" => String.downcase(mesh_blake3)}]
      })

    {:ok, %{bytes: commit_bytes, hash_hex: commit_hash}} =
      VcsObject.encode_commit(%{
        "tree" => tree_hash,
        "parents" => [parent_commit.commit_hash],
        "generation" => parent_commit.generation + 1,
        "author_sub" => author,
        "created_at_ms" => created_at_ms,
        "message" => message
      })

    %{
      tree_hash: tree_hash,
      tree_bytes: tree_bytes,
      commit_hash: commit_hash,
      commit_bytes: commit_bytes,
      mesh_blake3: String.downcase(mesh_blake3)
    }
  end

  test "negotiate reports missing objects and blobs (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "neg-base")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "neg-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    fake_obj = String.duplicate("a", 64)
    fake_blob = String.duplicate("b", 64)

    assert {:ok,
            %{
              missing_object_hashes: missing_objs,
              missing_blob_blake3s: missing_blobs
            }} =
             Vcs.negotiate(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               object_hashes: [child.tree_hash, child.commit_hash, c1.commit_hash, fake_obj],
               blob_blake3s: [child.mesh_blake3, fake_blob]
             })

    assert child.tree_hash in missing_objs
    assert child.commit_hash in missing_objs
    refute c1.commit_hash in missing_objs
    assert fake_obj in missing_objs
    refute child.mesh_blake3 in missing_blobs
    assert fake_blob in missing_blobs
  end

  test "push advances main with objects, repo_blobs, Authorship (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-ok")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    assert {:ok, %VcsRef{commit_hash: tip}} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes}
               ]
             })

    assert tip == child.commit_hash
    assert {:ok, %VcsRef{commit_hash: ^tip}} = Vcs.get_ref(repo.repo_id, "main")
    assert Vcs.commit_in_repo?(repo.repo_id, child.commit_hash)

    assert %RepoBlob{} =
             Assets.Repo.get_by!(RepoBlob, repo_id: repo.repo_id, blake3: child.mesh_blake3)

    assert %Cert{cert_kind: "commit", target_id: ^tip} = authorship_cert!(child.commit_hash)

    assert {:ok, blobs} = ItemRef.resolve({:repo_rev, repo.repo_id, "main"}, @sub_a)
    assert Enum.any?(blobs, &(&1.blake3 == child.mesh_blake3))
  end

  test "push rejects non-owner, cas mismatch, author mismatch, protected (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-deny")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-deny-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    attrs = %{
      ref_name: "main",
      old_commit_hash: c1.commit_hash,
      new_commit_hash: child.commit_hash,
      objects: [
        %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
        %{hash_hex: child.commit_hash, bytes: child.commit_bytes}
      ]
    }

    assert {:error, :forbidden} = Vcs.push(repo.repo_id, @sub_b, attrs)

    assert {:error, :cas_mismatch} =
             Vcs.push(repo.repo_id, @sub_a, %{attrs | old_commit_hash: String.duplicate("0", 64)})

    bad_author = encode_push_child(c1, mesh.blake3, author_sub: @sub_b, message: "evil")

    assert {:error, :author_mismatch} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: bad_author.commit_hash,
               objects: [
                 %{hash_hex: bad_author.tree_hash, bytes: bad_author.tree_bytes},
                 %{hash_hex: bad_author.commit_hash, bytes: bad_author.commit_bytes}
               ]
             })

    assert {:ok, _} = Vcs.set_branch_protection(repo.repo_id, @sub_a, "main", true)

    assert {:error, :ref_protected} = Vcs.push(repo.repo_id, @sub_a, attrs)
    assert {:error, :ref_protected} = Vcs.advance_branch(repo.repo_id, "main", child.commit_hash)

    assert {:ok, %VcsRef{commit_hash: tip}} = Vcs.get_ref(repo.repo_id, "main")
    assert tip == c1.commit_hash
  end

  test "push create-on-push new branch when old is nil (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-new-branch")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-nb-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    assert {:ok, %VcsRef{name: "feature", commit_hash: tip, kind: "branch"}} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "feature",
               old_commit_hash: nil,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes}
               ]
             })

    assert tip == child.commit_hash
  end

  test "push rejects missing child grant (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-no-grant")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-ng-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    grant =
      Assets.Repo.get_by!(Assets.Blobs.BlobGrant, grantee_sub: @sub_a, blake3: child.mesh_blake3)

    {:ok, _} = Assets.Repo.delete(grant)

    assert {:error, :no_grant} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes}
               ]
             })
  end

  test "push rejects non-FF and missing CAS blob (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-ff-blob")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-ff-mesh", "application/octet-stream")

    # Sibling root — not a descendant of c1
    {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "c0", "blake3" => String.downcase(mesh.blake3)}]
      })

    {:ok, %{bytes: commit_bytes, hash_hex: sibling_hash}} =
      VcsObject.encode_commit(%{
        "tree" => tree_hash,
        "parents" => [],
        "generation" => 1,
        "author_sub" => @sub_a,
        "created_at_ms" => System.system_time(:millisecond),
        "message" => "sibling-root"
      })

    assert {:error, :not_fast_forward} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: sibling_hash,
               objects: [
                 %{hash_hex: tree_hash, bytes: tree_bytes},
                 %{hash_hex: sibling_hash, bytes: commit_bytes}
               ]
             })

    child = encode_push_child(c1, mesh.blake3)
    Assets.Repo.get!(Assets.Blobs.CasObject, child.mesh_blake3) |> Assets.Repo.delete!()

    assert {:error, {:missing_blob, _}} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes}
               ]
             })
  end

  test "push rejects unreachable commit in payload (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-unreach")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-ur-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    {:ok, %{bytes: ghost_tree_bytes, hash_hex: ghost_tree}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "g0", "blake3" => String.downcase(mesh.blake3)}]
      })

    {:ok, %{bytes: ghost_commit_bytes, hash_hex: ghost_hash}} =
      VcsObject.encode_commit(%{
        "tree" => ghost_tree,
        "parents" => [],
        "generation" => 1,
        "author_sub" => @sub_a,
        "created_at_ms" => System.system_time(:millisecond) + 1,
        "message" => "ghost"
      })

    assert {:error, {:unreachable_commit, ^ghost_hash}} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes},
                 %{hash_hex: ghost_tree, bytes: ghost_tree_bytes},
                 %{hash_hex: ghost_hash, bytes: ghost_commit_bytes}
               ]
             })
  end

  test "push drops orphan trees and backfills origins on joining (x00cc)" do
    {node, _} = build_prefab_node(@sub_a, "push-orphan-join")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Assets.Blobs.upload(@sub_a, "push-oj-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    {:ok, %{bytes: orphan_bytes, hash_hex: orphan_tree}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "orphan", "blake3" => String.downcase(mesh.blake3)}]
      })

    assert {:ok, _} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "main",
               old_commit_hash: c1.commit_hash,
               new_commit_hash: child.commit_hash,
               objects: [
                 %{hash_hex: child.tree_hash, bytes: child.tree_bytes},
                 %{hash_hex: child.commit_hash, bytes: child.commit_bytes},
                 %{hash_hex: orphan_tree, bytes: orphan_bytes}
               ]
             })

    assert {:error, :not_found} = Vcs.get_object(orphan_tree)
    assert {:ok, %Object{}} = Vcs.get_object(child.tree_hash)

    # Simulate joining-only: commit remains global, leave this repo's index.
    Assets.Repo.get_by!(Assets.Vcs.RepoCommit,
      repo_id: repo.repo_id,
      commit_hash: child.commit_hash
    )
    |> Assets.Repo.delete!()

    Assets.Repo.get!(BlobOrigin, child.mesh_blake3) |> Assets.Repo.delete!()

    assert {:ok, _} =
             Vcs.push(repo.repo_id, @sub_a, %{
               ref_name: "feature",
               old_commit_hash: nil,
               new_commit_hash: child.commit_hash,
               objects: []
             })

    assert %Commit{author_sub: author} = Assets.Repo.get!(Commit, child.commit_hash)

    assert %BlobOrigin{first_commit_hash: tip, author_sub: ^author} =
             Assets.Repo.get!(BlobOrigin, child.mesh_blake3)

    assert tip == child.commit_hash
    assert author == @sub_a
    assert Vcs.commit_in_repo?(repo.repo_id, child.commit_hash)
  end
end
