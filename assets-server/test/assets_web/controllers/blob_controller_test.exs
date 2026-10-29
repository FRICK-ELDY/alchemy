defmodule AssetsWeb.BlobControllerTest do
  use AssetsWeb.ConnCase, async: false

  alias Assets.Blobs
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.Quota.QuotaLedger
  alias Assets.Repo
  alias Assets.TestAuthHelpers

  @sub_a "22222222-2222-2222-2222-222222222222"
  @sub_b "33333333-3333-3333-3333-333333333333"

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    Application.delete_env(:assets, :fail_finalize_blake3)
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    on_exit(fn -> Application.delete_env(:assets, :fail_finalize_blake3) end)
    %{signer: signer, sub: @sub_a}
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

    assert %{"used_bytes" => 17, "reserved_bytes" => 0, "limit_bytes" => 1024} =
             json_response(conn, 200)

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

    assert %{"used_bytes" => 17, "reserved_bytes" => 0} = json_response(conn, 200)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body
  end

  test "cross-owner same bytes creates second ref and charges both quotas", %{
    conn: conn,
    signer: signer
  } do
    body = "shared-cas-bytes"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "status" => "created"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => ^blake3, "status" => "created"} = json_response(conn, 201)

    assert %{used_bytes: 16, reserved_bytes: 0} = Blobs.quota(@sub_a)
    assert %{used_bytes: 16, reserved_bytes: 0} = Blobs.quota(@sub_b)
    assert Repo.aggregate(CasObject, :count) == 1
    assert CAS.exists?(blake3)
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

  test "conditional quota: dual upload callers yield exactly one success", %{sub: sub} do
    # Two 600-byte uploads against 1024 limit: only one may succeed (x002d / x003f reserve).
    # ConnCase async: false → shared sandbox (one connection): Tasks serialize,
    # so this asserts the API contract, not a true race on the conditional UPDATE.
    tasks =
      for _ <- 1..2 do
        Task.async(fn ->
          Blobs.upload(sub, :crypto.strong_rand_bytes(600), "application/octet-stream")
        end)
      end

    results = Enum.map(tasks, &Task.await(&1, 10_000))
    oks = Enum.count(results, &match?({:ok, _, :created}, &1))
    errs = Enum.count(results, &match?({:error, :quota_exceeded}, &1))
    assert oks == 1
    assert errs == 1
    assert %{used_bytes: used, reserved_bytes: 0, limit_bytes: 1024} = Blobs.quota(sub)
    assert used == 600
  end

  test "finalize failure releases reserved and leaves CAS orphan (no delete)", %{sub: sub} do
    # Compensate must not CAS.delete — concurrent same-blake3 putters share bytes.
    data = "orphan-compensate-bytes"
    blake3 = Blobs.blake3_hex(data)
    Application.put_env(:assets, :fail_finalize_blake3, blake3)

    assert {:error, :forced_finalize_failure} =
             Blobs.upload(sub, data, "application/octet-stream")

    assert %{used_bytes: 0, reserved_bytes: 0} = Blobs.quota(sub)
    assert Repo.get(CasObject, blake3) == nil
    # Orphan bytes may remain; GC is out of scope for x003f.
    assert CAS.exists?(blake3)
  end

  test "finalize failure after peer object keeps shared CAS and peer quota", %{
    conn: conn,
    signer: signer
  } do
    body = "keep-shared-object"
    blake3 = Blobs.blake3_hex(body)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"status" => "created"} = json_response(conn, 201)
    assert CAS.exists?(blake3)

    Application.put_env(:assets, :fail_finalize_blake3, blake3)

    assert {:error, :forced_finalize_failure} =
             Blobs.upload(@sub_b, body, "text/plain")

    assert Blobs.quota(@sub_a) == %{
             used_bytes: byte_size(body),
             reserved_bytes: 0,
             limit_bytes: 1024
           }

    assert Blobs.quota(@sub_b) == %{used_bytes: 0, reserved_bytes: 0, limit_bytes: 1024}
    assert Repo.get(CasObject, blake3)
    assert CAS.exists?(blake3)
  end

  test "cross-owner concurrent same bytes both create refs", %{sub: _sub} do
    # ConnCase async:false → shared Ecto sandbox (one connection): Tasks serialize.
    # This is a multi-owner success contract test, not a true parallel race on put/compensate.
    body = "concurrent-cross-owner-bytes"
    blake3 = Blobs.blake3_hex(body)

    tasks =
      for owner <- [@sub_a, @sub_b] do
        Task.async(fn -> Blobs.upload(owner, body, "text/plain") end)
      end

    results = Enum.map(tasks, &Task.await(&1, 10_000))
    assert Enum.all?(results, &match?({:ok, %{blake3: ^blake3}, :created}, &1))
    assert Repo.aggregate(CasObject, :count) == 1
    assert CAS.exists?(blake3)
    assert Blobs.quota(@sub_a).used_bytes == byte_size(body)
    assert Blobs.quota(@sub_b).used_bytes == byte_size(body)
    assert Blobs.quota(@sub_a).reserved_bytes == 0
    assert Blobs.quota(@sub_b).reserved_bytes == 0
  end

  test "missing CAS body is repaired when CasObject already exists", %{sub: sub} do
    body = "repair-missing-cas-body"
    assert {:ok, object, :created} = Blobs.upload(sub, body, "text/plain")
    assert :ok = CAS.delete(object.blake3)
    refute CAS.exists?(object.blake3)

    assert {:ok, repaired, :existing} = Blobs.upload(sub, body, "text/plain")
    assert repaired.blake3 == object.blake3
    assert CAS.exists?(object.blake3)

    assert Blobs.quota(sub) == %{
             used_bytes: byte_size(body),
             reserved_bytes: 0,
             limit_bytes: 1024
           }
  end

  test "upload with application/json content-type still reads raw body", %{
    conn: conn,
    signer: signer,
    sub: sub
  } do
    # Use opaque bytes so ConnTest does not JSON-encode the body.
    body = <<0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11>>

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "application/json")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "size" => 12, "status" => "created"} = json_response(conn, 201)
    assert blake3 == Blobs.blake3_hex(body)
  end

  test "health", %{conn: conn} do
    conn = get(conn, "/health")
    assert %{"status" => "ok", "service" => "assets-server"} = json_response(conn, 200)
  end

  test "blake3 helper", %{sub: sub} do
    data = "x"
    assert {:ok, object, :created} = Blobs.upload(sub, data, "text/plain")
    assert object.blake3 == Blobs.blake3_hex(data)
    assert %QuotaLedger{reserved_bytes: 0} = Repo.get(QuotaLedger, sub)
  end
end
