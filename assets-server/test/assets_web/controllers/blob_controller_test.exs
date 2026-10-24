defmodule AssetsWeb.BlobControllerTest do
  use AssetsWeb.ConnCase, async: false

  alias Assets.Blobs
  alias Assets.TestAuthHelpers

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer, sub: "22222222-2222-2222-2222-222222222222"}
  end

  test "rejects upload without auth", %{conn: conn} do
    conn =
      conn
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", "hello")

    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  test "upload download and dedupe quota", %{conn: conn, signer: signer, sub: sub} do
    body = "alchemy-cas-bytes"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "size" => 17, "status" => "created"} = json_response(conn, 201)
    assert String.length(blake3) == 64

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> get("/api/v1/quota")

    assert %{"used_bytes" => 17, "limit_bytes" => 1024} = json_response(conn, 200)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => ^blake3, "status" => "existing"} = json_response(conn, 200)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> get("/api/v1/quota")

    assert %{"used_bytes" => 17} = json_response(conn, 200)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body
  end

  test "quota exceeded", %{conn: conn, signer: signer, sub: sub} do
    # test limit is 1024 bytes
    big = :crypto.strong_rand_bytes(600)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "application/octet-stream")
      |> post("/api/v1/blobs", big)

    assert %{"blake3" => _} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "application/octet-stream")
      |> post("/api/v1/blobs", :crypto.strong_rand_bytes(600))

    assert %{"error" => "quota_exceeded"} = json_response(conn, 413)
  end

  test "health", %{conn: conn} do
    conn = get(conn, "/health")
    assert %{"status" => "ok", "service" => "assets-server"} = json_response(conn, 200)
  end

  test "blake3 helper", %{sub: sub} do
    data = "x"
    assert {:ok, blob, :created} = Blobs.upload(sub, data, "text/plain")
    assert blob.blake3 == Blobs.blake3_hex(data)
  end
end
