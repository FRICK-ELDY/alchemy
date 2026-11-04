defmodule AssetsWeb.BlobControllerTest do
  use AssetsWeb.ConnCase, async: false

  alias Assets.Blobs
  alias Assets.Blobs.BlobGrant
  alias Assets.Blobs.CasObject
  alias Assets.CAS
  alias Assets.Quota.QuotaLedger
  alias Assets.Repo
  alias Assets.TestAuthHelpers
  alias Assets.TestPlaceReadHelpers

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

  test "quarantine rejects invalid GLB header with stable public reason", %{
    conn: conn,
    signer: signer,
    sub: sub
  } do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "model/gltf-binary; charset=binary")
      |> post("/api/v1/blobs", "not-a-glb")

    assert %{"error" => "quarantine", "reason" => "invalid_glb_header"} =
             json_response(conn, 422)
  end

  test "quarantine maps unknown worker reasons to rejected without inspect leak", %{
    conn: conn,
    signer: signer,
    sub: sub
  } do
    previous = Application.get_env(:assets, :quarantine_worker)

    Application.put_env(
      :assets,
      :quarantine_worker,
      AssetsWeb.BlobControllerTest.RejectWithOpaqueWorker
    )

    on_exit(fn ->
      if previous do
        Application.put_env(:assets, :quarantine_worker, previous)
      else
        Application.delete_env(:assets, :quarantine_worker)
      end
    end)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "application/octet-stream")
      |> post("/api/v1/blobs", "opaque-reject")

    body = json_response(conn, 422)
    assert body == %{"error" => "quarantine", "reason" => "rejected"}
    refute inspect(body) =~ "secret_threshold"
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

  test "default upload: owner Grant GET 200; other user needs PlaceRead (x006c/x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "room-mesh-capability"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "status" => "created"} = body_json = json_response(conn, 201)
    refute Map.has_key?(body_json, "access")

    assert %BlobGrant{grant_type: "owned"} =
             Repo.get_by(BlobGrant, grantee_sub: @sub_a, blake3: blake3)

    assert %CasObject{blake3: ^blake3} = Repo.get(CasObject, blake3)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "CasObject GET allowed via PlaceRead (x006c)", %{
    conn: conn,
    signer: signer
  } do
    body = "authenticated-place-read-scene"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("22", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)

    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [blake3])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_b,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body
  end

  test "upload: owner GET 200, other user 403", %{
    conn: conn,
    signer: signer
  } do
    body = "private-drive-bytes"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "status" => "created"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "GET allowed via PlaceRead covering scene hash (x006b)", %{
    conn: conn,
    signer: signer
  } do
    body = "place-read-scene-bytes"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("11", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)

    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [blake3])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_b,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body
  end

  test "PlaceRead sub mismatch is forbidden (x006b)", %{conn: conn, signer: signer} do
    body = "place-read-sub-mismatch"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("33", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)
    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [blake3])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_a,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "PlaceRead does not cover hash outside seal allowlist (x006b)", %{
    conn: conn,
    signer: signer
  } do
    body = "place-read-uncovered"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("44", 32)
    other = String.duplicate("55", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)
    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [other])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_b,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "PlaceRead with non-empty extra_blake3 claim is rejected (x006b)", %{
    conn: conn,
    signer: signer
  } do
    body = "place-read-extra-disabled"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("66", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)
    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [blake3])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_b,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest,
        extra_blake3: [blake3]
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "inventory PUT: owner GET 200, other user 403", %{
    conn: conn,
    signer: signer
  } do
    body = "inventory-private-bytes"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/inventory/blobs", body)

    assert %{"blake3" => blake3, "status" => "created"} = json_response(conn, 201)

    assert %BlobGrant{grant_type: "owned"} =
             Repo.get_by(BlobGrant, grantee_sub: @sub_a, blake3: blake3)

    assert %CasObject{blake3: ^blake3} = Repo.get(CasObject, blake3)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "obsolete x-alchemy-blob-access header is ignored on inventory PUT (x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "inventory-ignores-access-header"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> put_req_header("x-alchemy-blob-access", "authenticated")
      |> post("/api/v1/inventory/blobs", body)

    assert %{"blake3" => _blake3, "status" => "created"} = json_response(conn, 201)
  end

  test "room-mesh and inventory paths stay separated for different bodies (x006c)", %{
    conn: conn,
    signer: signer
  } do
    mesh = "room-mesh-still-authenticated"
    inventory = "inventory-private-sibling"
    %{signer: world_signer} = TestPlaceReadHelpers.install_test_world_jwks!()
    manifest = String.duplicate("33", 32)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", mesh)

    assert %{"blake3" => mesh_blake3, "status" => "created"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/inventory/blobs", inventory)

    assert %{"blake3" => inv_blake3, "status" => "created"} = json_response(conn, 201)

    assert mesh_blake3 != inv_blake3

    # Case B: Bearer alone cannot read room mesh or inventory.
    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{mesh_blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{inv_blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)

    TestPlaceReadHelpers.seed_scene_hashes("local", 1, manifest, [mesh_blake3])

    place_read =
      TestPlaceReadHelpers.place_read_token(world_signer,
        sub: @sub_b,
        room_id: "local",
        revision: 1,
        manifest_blake3: manifest
      )

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{mesh_blake3}")

    assert response(conn, 200) == mesh

    # PlaceRead covers seal-time scene only — inventory stays private.
    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("x-alchemy-place-read", place_read)
      |> get("/api/v1/blobs/#{inv_blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end

  test "mesh then inventory PUT coexist on same blake3 without conflict (x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "mesh-inventory-same-bytes-coexist"
    assert_cross_path_coexist(conn, signer, body, :blobs_first)
  end

  test "inventory then mesh PUT coexist on same blake3 without conflict (x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "inventory-mesh-same-bytes-coexist"
    assert_cross_path_coexist(conn, signer, body, :inventory_first)
  end

  test "/blobs and /inventory/blobs upload responses match for same body (x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "path-alias-same-behaviour"

    conn_a =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3, "size" => size, "mime" => mime, "status" => "created"} =
             json_response(conn_a, 201)

    conn_b =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/inventory/blobs", body)

    assert %{
             "blake3" => ^blake3,
             "size" => ^size,
             "mime" => ^mime,
             "status" => "created"
           } = body_b = json_response(conn_b, 201)

    refute Map.has_key?(body_b, "access")
  end

  test "rejects GET without auth", %{conn: conn, signer: signer, sub: sub} do
    body = "need-auth-to-get"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "text/plain")
      |> post("/api/v1/blobs", body)

    assert %{"blake3" => blake3} = json_response(conn, 201)

    conn = build_conn() |> get("/api/v1/blobs/#{blake3}")
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  test "obsolete x-alchemy-blob-access header is ignored on /blobs (x006f)", %{
    conn: conn,
    signer: signer,
    sub: sub
  } do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put_req_header("content-type", "text/plain")
      |> put_req_header("x-alchemy-blob-access", "public")
      |> post("/api/v1/blobs", "header-ignored")

    assert %{"blake3" => _blake3, "status" => "created"} = json_response(conn, 201)
  end

  test "cross-owner same bytes: second owner gets Grant and can GET (x006f)", %{
    conn: conn,
    signer: signer
  } do
    body = "shared-private-bytes"

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

    assert %BlobGrant{grant_type: "owned"} =
             Repo.get_by(BlobGrant, grantee_sub: @sub_b, blake3: blake3)

    assert Blobs.quota(@sub_b).used_bytes == byte_size(body)
    assert Blobs.quota(@sub_b).reserved_bytes == 0

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body
  end

  defp assert_cross_path_coexist(conn, signer, body, order) do
    {first_path, second_path} =
      case order do
        :blobs_first -> {"/api/v1/blobs", "/api/v1/inventory/blobs"}
        :inventory_first -> {"/api/v1/inventory/blobs", "/api/v1/blobs"}
      end

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> put_req_header("content-type", "text/plain")
      |> post(first_path, body)

    assert %{"blake3" => blake3, "status" => "created"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> put_req_header("content-type", "text/plain")
      |> post(second_path, body)

    assert %{"blake3" => ^blake3, "status" => "created"} = json_response(conn, 201)

    assert %CasObject{blake3: ^blake3} = Repo.get(CasObject, blake3)
    assert %BlobGrant{} = Repo.get_by(BlobGrant, grantee_sub: @sub_a, blake3: blake3)
    assert %BlobGrant{} = Repo.get_by(BlobGrant, grantee_sub: @sub_b, blake3: blake3)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_a)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, @sub_b)
      |> get("/api/v1/blobs/#{blake3}")

    assert response(conn, 200) == body

    stranger = "44444444-4444-4444-4444-444444444444"

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, stranger)
      |> get("/api/v1/blobs/#{blake3}")

    assert %{"error" => "forbidden"} = json_response(conn, 403)
  end
end

defmodule AssetsWeb.BlobControllerTest.RejectWithOpaqueWorker do
  @behaviour Assets.Quarantine.Worker

  @impl true
  def inspect_blob(_blake3, _data, _mime) do
    {:quarantine, {:secret_threshold, 0.87, %{internal: true}}}
  end
end
