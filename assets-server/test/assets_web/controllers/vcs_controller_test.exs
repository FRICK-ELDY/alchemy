defmodule AssetsWeb.VcsControllerTest do
  use AssetsWeb.ConnCase, async: false, cas: true

  alias Assets.Blobs
  alias Assets.Drive
  alias Assets.Prefab.Payload
  alias Assets.TestAuthHelpers
  alias Assets.Vcs
  alias Assets.Vcs.Cert
  alias Assets.VcsObject

  @sub_a "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
  @sub_b "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

  setup do
    Cert.install_test_key!()
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer}
  end

  defp build_prefab(name) do
    {:ok, mesh, _} = Blobs.upload(@sub_a, "http-vcs-#{name}", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [%{local_id: "c0", blake3: mesh.blake3, trs: nil}])

    {:ok, node} = Drive.save_prefab(@sub_a, name, payload)
    {node, mesh}
  end

  defp encode_push_child(parent_commit, mesh_blake3) do
    {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "c0", "blake3" => String.downcase(mesh_blake3)}]
      })

    {:ok, %{bytes: commit_bytes, hash_hex: commit_hash}} =
      VcsObject.encode_commit(%{
        "tree" => tree_hash,
        "parents" => [parent_commit.commit_hash],
        "generation" => parent_commit.generation + 1,
        "author_sub" => @sub_a,
        "created_at_ms" => System.system_time(:millisecond),
        "message" => "http-push"
      })

    %{
      tree_hash: tree_hash,
      tree_bytes: tree_bytes,
      commit_hash: commit_hash,
      commit_bytes: commit_bytes,
      mesh_blake3: String.downcase(mesh_blake3)
    }
  end

  test "negotiate and push over HTTP (x00cd)", %{conn: conn, signer: signer} do
    {node, _} = build_prefab("http-ok")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Blobs.upload(@sub_a, "http-push-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    fake = String.duplicate("a", 64)

    neg =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/negotiate",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: c1.commit_hash,
          new_commit_hash: child.commit_hash,
          object_hashes: [child.tree_hash, child.commit_hash, fake],
          blob_blake3s: [child.mesh_blake3]
        })
      )

    assert %{
             "missing_object_hashes" => missing_objs,
             "missing_blob_blake3s" => missing_blobs
           } = json_response(neg, 200)

    assert child.tree_hash in missing_objs
    assert child.commit_hash in missing_objs
    assert fake in missing_objs
    assert missing_blobs == []

    push =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: c1.commit_hash,
          new_commit_hash: child.commit_hash,
          objects: [
            %{hash_hex: child.tree_hash, bytes_b64: Base.encode64(child.tree_bytes)},
            %{hash_hex: child.commit_hash, bytes_b64: Base.encode64(child.commit_bytes)}
          ]
        })
      )

    assert %{"ref" => %{"name" => "main", "commit_hash" => tip}} = json_response(push, 200)
    assert tip == child.commit_hash
  end

  test "push rejects non-owner, protected, non-FF (x00cd)", %{conn: conn, signer: signer} do
    {node, _} = build_prefab("http-deny")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Blobs.upload(@sub_a, "http-deny-mesh", "application/octet-stream")
    child = encode_push_child(c1, mesh.blake3)

    body =
      Jason.encode!(%{
        ref_name: "main",
        old_commit_hash: c1.commit_hash,
        new_commit_hash: child.commit_hash,
        objects: [
          %{hash_hex: child.tree_hash, bytes_b64: Base.encode64(child.tree_bytes)},
          %{hash_hex: child.commit_hash, bytes_b64: Base.encode64(child.commit_bytes)}
        ]
      })

    forbidden =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/vcs/repos/#{repo.repo_id}/push", body)

    assert %{"error" => "forbidden"} = json_response(forbidden, 403)

    assert {:ok, _} = Vcs.set_branch_protection(repo.repo_id, @sub_a, "main", true)

    protected =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/vcs/repos/#{repo.repo_id}/push", body)

    assert %{"error" => "ref_protected"} = json_response(protected, 403)

    assert {:ok, _} = Vcs.set_branch_protection(repo.repo_id, @sub_a, "main", false)

    {:ok, %{bytes: tree_bytes, hash_hex: tree_hash}} =
      VcsObject.encode_tree(%{
        "children" => [%{"local_id" => "c0", "blake3" => String.downcase(mesh.blake3)}]
      })

    {:ok, %{bytes: commit_bytes, hash_hex: sibling}} =
      VcsObject.encode_commit(%{
        "tree" => tree_hash,
        "parents" => [],
        "generation" => 1,
        "author_sub" => @sub_a,
        "created_at_ms" => System.system_time(:millisecond),
        "message" => "sibling"
      })

    non_ff =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: c1.commit_hash,
          new_commit_hash: sibling,
          objects: [
            %{hash_hex: tree_hash, bytes_b64: Base.encode64(tree_bytes)},
            %{hash_hex: sibling, bytes_b64: Base.encode64(commit_bytes)}
          ]
        })
      )

    assert %{"error" => "not_fast_forward"} = json_response(non_ff, 409)
  end

  test "push two hops and multi-commit joining (x00cd)", %{conn: conn, signer: signer} do
    {node, _} = build_prefab("http-two-hop")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)
    {:ok, mesh, _} = Blobs.upload(@sub_a, "http-two-mesh", "application/octet-stream")
    child1 = encode_push_child(c1, mesh.blake3)

    hop1 =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: c1.commit_hash,
          new_commit_hash: child1.commit_hash,
          objects: [
            %{hash_hex: child1.tree_hash, bytes_b64: Base.encode64(child1.tree_bytes)},
            %{hash_hex: child1.commit_hash, bytes_b64: Base.encode64(child1.commit_bytes)}
          ]
        })
      )

    assert %{"ref" => %{"commit_hash" => tip1}} = json_response(hop1, 200)
    assert tip1 == child1.commit_hash

    parent = %{commit_hash: child1.commit_hash, generation: c1.generation + 1}
    child2 = encode_push_child(parent, mesh.blake3)
    parent2 = %{commit_hash: child2.commit_hash, generation: c1.generation + 2}
    child3 = encode_push_child(parent2, mesh.blake3)

    # One HTTP push carrying two new commits (Rust multi-commit CAS shape).
    # Tree may be identical for both children — send each hash once (split_verified).
    multi =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: child1.commit_hash,
          new_commit_hash: child3.commit_hash,
          objects: [
            %{hash_hex: child2.tree_hash, bytes_b64: Base.encode64(child2.tree_bytes)},
            %{hash_hex: child2.commit_hash, bytes_b64: Base.encode64(child2.commit_bytes)},
            %{hash_hex: child3.commit_hash, bytes_b64: Base.encode64(child3.commit_bytes)}
          ]
        })
      )

    assert %{"ref" => %{"commit_hash" => tip3}} = json_response(multi, 200)
    assert tip3 == child3.commit_hash

    stale_old =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: c1.commit_hash,
          new_commit_hash: child3.commit_hash,
          objects: []
        })
      )

    assert %{"error" => "cas_mismatch"} = json_response(stale_old, 409)
  end

  test "negotiate rejects unauthenticated and bad attrs (x00cd)", %{conn: conn, signer: signer} do
    {node, _} = build_prefab("http-auth")
    {:ok, %{repo: repo, commit: c1}} = Vcs.promote_prefab(@sub_a, node.node_id)

    no_auth =
      conn
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/negotiate",
        Jason.encode!(%{
          ref_name: "main",
          new_commit_hash: c1.commit_hash,
          object_hashes: [],
          blob_blake3s: []
        })
      )

    assert no_auth.status == 401

    bad_json =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/vcs/repos/#{repo.repo_id}/negotiate", "{not-json")

    assert %{"error" => "invalid_json"} = json_response(bad_json, 400)

    bad_hash =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/negotiate",
        Jason.encode!(%{
          ref_name: "main",
          new_commit_hash: "short",
          object_hashes: [],
          blob_blake3s: []
        })
      )

    assert %{"error" => "invalid_attr:new_commit_hash"} = json_response(bad_hash, 422)

    bad_old =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/push",
        Jason.encode!(%{
          ref_name: "main",
          old_commit_hash: "not-64",
          new_commit_hash: c1.commit_hash,
          objects: []
        })
      )

    assert %{"error" => "invalid_attr:old_commit_hash"} = json_response(bad_old, 422)

    oversized =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/vcs/repos/#{repo.repo_id}/negotiate",
        String.duplicate("x", 70_000)
      )

    assert %{"error" => "too_large"} = json_response(oversized, 413)
  end
end
