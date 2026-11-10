defmodule AssetsWeb.DriveControllerTest do
  use AssetsWeb.ConnCase, async: false, cas: true

  alias Assets.Blobs
  alias Assets.Blobs.BlobGrant
  alias Assets.Drive
  alias Assets.Prefab.Payload
  alias Assets.Repo
  alias Assets.TestAuthHelpers

  @sub_a "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
  @sub_b "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer}
  end

  test "lists owner drive nodes", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "mesh-bytes", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> get("/api/v1/drive/nodes")

    assert %{"nodes" => nodes} = json_response(conn, 200)
    assert Enum.any?(nodes, fn n -> n["node_id"] == node.node_id and n["kind"] == "blob" end)
  end

  test "take_out returns blobs for owner with grant", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "take-out-mesh", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{node_id: node.node_id}))

    assert %{"node" => %{"node_id" => nid, "kind" => "blob"}, "blobs" => [%{"blake3" => b3}]} =
             json_response(conn, 200)

    assert nid == node.node_id
    assert b3 == cas_obj.blake3
  end

  test "take_out expands prefab children", %{conn: conn, signer: signer} do
    {:ok, mesh_a, _} = Blobs.upload(@sub_a, "prefab-mesh-a", "application/octet-stream")
    {:ok, mesh_b, _} = Blobs.upload(@sub_a, "prefab-mesh-b", "application/octet-stream")

    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "a", blake3: mesh_a.blake3, trs: nil},
        %{
          local_id: "b",
          blake3: mesh_b.blake3,
          trs: %{
            translation: [1.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0]
          }
        }
      ])

    {:ok, node} = Drive.save_prefab(@sub_a, "Multi", payload)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{node_id: node.node_id}))

    assert %{"node" => %{"kind" => "prefab"}, "blobs" => blobs} = json_response(conn, 200)
    assert length(blobs) == 2
    assert Enum.at(blobs, 0)["blake3"] == mesh_a.blake3
    assert Enum.at(blobs, 0)["local_id"] == "a"
    assert Enum.at(blobs, 1)["blake3"] == mesh_b.blake3
    assert Enum.at(blobs, 1)["local_id"] == "b"
    assert Enum.at(blobs, 1)["trs"]["translation"] == [1.0, 0.0, 0.0]
  end

  test "take_out other owner is not_found", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "private", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{node_id: node.node_id}))

    assert %{"error" => "not_found"} = json_response(conn, 404)
  end

  test "take_out without grant is forbidden", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub_a, "orphan-grant", "application/octet-stream")
    {:ok, node} = Drive.create_blob(@sub_a, "mesh", cas_obj.blake3)

    grant = Repo.get_by!(BlobGrant, grantee_sub: @sub_a, blake3: cas_obj.blake3)
    {:ok, _} = Repo.delete(grant)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{node_id: node.node_id}))

    assert %{"error" => "no_grant"} = json_response(conn, 403)
  end

  test "take_out without node_id is 422", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{}))

    assert %{"error" => "node_id_required"} = json_response(conn, 422)
  end

  test "put_back saves single-child prefab", %{conn: conn, signer: signer} do
    blake3 = String.duplicate("a", 64)

    body = %{
      name: "putback-item",
      payload: %{
        schema_version: 1,
        children: [%{local_id: "mesh_0", blake3: blake3}]
      }
    }

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/put_back", Jason.encode!(body))

    assert %{
             "node" => %{
               "kind" => "prefab",
               "name" => "putback-item",
               "blake3" => payload_blake3
             }
           } = json_response(conn, 200)

    assert byte_size(payload_blake3) == 64
  end

  test "put_back upserts same name", %{conn: conn, signer: signer} do
    blake3_a = String.duplicate("a", 64)
    blake3_b = String.duplicate("b", 64)

    body1 = %{
      name: "shared-putback",
      payload: %{schema_version: 1, children: [%{local_id: "c0", blake3: blake3_a}]}
    }

    conn1 =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/put_back", Jason.encode!(body1))

    assert %{"node" => %{"node_id" => id1, "blake3" => b1}} = json_response(conn1, 200)

    body2 = %{
      name: "shared-putback",
      payload: %{schema_version: 1, children: [%{local_id: "c1", blake3: blake3_b}]}
    }

    conn2 =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/put_back", Jason.encode!(body2))

    assert %{"node" => %{"node_id" => id2, "blake3" => b2}} = json_response(conn2, 200)
    assert id1 == id2
    assert b1 != b2
  end

  test "put_back without name is 422", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/drive/put_back",
        Jason.encode!(%{payload: %{schema_version: 1, children: []}})
      )

    assert %{"error" => "name_required"} = json_response(conn, 422)
  end

  test "put_back name too long is 422", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/drive/put_back",
        Jason.encode!(%{
          name: String.duplicate("n", 256),
          payload: %{schema_version: 1, children: []}
        })
      )

    assert %{"error" => "name_too_long"} = json_response(conn, 422)
  end

  test "put_back invalid payload is 422 without internal detail", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/drive/put_back",
        Jason.encode!(%{name: "bad", payload: %{schema_version: 99, children: []}})
      )

    body = json_response(conn, 422)
    assert body["error"] == "invalid_payload"
    refute Map.has_key?(body, "detail")
  end

  test "put_back other parent is forbidden", %{conn: conn, signer: signer} do
    {:ok, folder} = Drive.create_folder(@sub_b, "Other")

    body = %{
      name: "nested",
      parent_id: folder.node_id,
      payload: %{schema_version: 1, children: []}
    }

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/put_back", Jason.encode!(body))

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "rejects without auth", %{conn: conn} do
    conn = get(conn, "/api/v1/drive/nodes")
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  test "put_back rejects without auth", %{conn: conn} do
    conn =
      conn
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/drive/put_back",
        Jason.encode!(%{name: "x", payload: %{schema_version: 1, children: []}})
      )

    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end
end
