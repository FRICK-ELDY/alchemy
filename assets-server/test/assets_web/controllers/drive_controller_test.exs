defmodule AssetsWeb.DriveControllerTest do
  use AssetsWeb.ConnCase, async: false

  alias Assets.Blobs
  alias Assets.Drive
  alias Assets.TestAuthHelpers

  @sub_a "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
  @sub_b "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
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

  test "take_out without node_id is 422", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/drive/take_out", Jason.encode!(%{}))

    assert %{"error" => "node_id_required"} = json_response(conn, 422)
  end

  test "rejects without auth", %{conn: conn} do
    conn = get(conn, "/api/v1/drive/nodes")
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end
end
