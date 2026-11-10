defmodule WorldWeb.RoomControllerTest do
  use WorldWeb.ConnCase, async: false

  alias Ecto.Adapters.SQL.Sandbox
  alias World.TestAuthHelpers

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer}
  end

  # x003e: every Room Token request must carry session_pub (base64url 32B).
  defp token_params(extra) when is_map(extra) do
    Map.put(
      extra,
      :session_pub,
      Base.url_encode64(:crypto.strong_rand_bytes(32), padding: false)
    )
  end

  defp sample_session_pub_bytes, do: :crypto.strong_rand_bytes(32)

  test "rejects room create without auth", %{conn: conn} do
    conn = post(conn, "/api/v1/rooms", %{room_id: "noauth"})
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  test "rejects room catalog without auth", %{conn: conn} do
    conn = get(conn, "/api/v1/rooms")
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  defp activate_host!(signer, sub, room_id) do
    assert %{"role" => "host", "state" => "active"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, sub)
             |> post("/api/v1/rooms/#{room_id}/tokens", token_params(%{role: "host"}))
             |> json_response(200)
  end

  test "lists visible rooms without secrets (x007c / x0080 active-only public)", %{
    conn: conn,
    signer: signer
  } do
    owner = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    guest = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

    assert %{"room_id" => "cat-public"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "cat-public", visibility: "public"})
             |> json_response(201)

    assert %{"room_id" => "cat-private"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "cat-private", visibility: "private"})
             |> json_response(201)

    # x0080: create leaves dormant — Public catalog empty until host activates.
    dormant_public =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> get("/api/v1/rooms")
      |> json_response(200)

    refute "cat-public" in Enum.map(dormant_public["rooms"], & &1["room_id"])

    activate_host!(signer, owner, "cat-public")
    activate_host!(signer, owner, "cat-private")

    body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> get("/api/v1/rooms")
      |> json_response(200)

    ids = Enum.map(body["rooms"], & &1["room_id"])
    assert "cat-public" in ids
    refute "cat-private" in ids

    public = Enum.find(body["rooms"], &(&1["room_id"] == "cat-public"))
    assert public["visibility"] == "public"
    assert public["owner_sub"] == owner
    assert public["role_hint"] == "joinable"
    assert public["state"] == "active"
    assert public["active_host_sub"] == owner
    refute Map.has_key?(public, "room_key")
    refute Map.has_key?(public, "wire_cap")
    refute Map.has_key?(public, "token")

    owner_body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> get("/api/v1/rooms")
      |> json_response(200)

    owner_ids = Enum.map(owner_body["rooms"], & &1["room_id"])
    assert "cat-public" in owner_ids
    assert "cat-private" in owner_ids

    private = Enum.find(owner_body["rooms"], &(&1["room_id"] == "cat-private"))
    assert private["role_hint"] == "owner"

    mine_body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> get("/api/v1/rooms?mine=1")
      |> json_response(200)

    assert mine_body["rooms"] == []
  end

  test "catalog keeps owned private room outside global public top-N (x007c)", %{
    conn: conn,
    signer: signer
  } do
    owner = "cccccccc-cccc-cccc-cccc-cccccccccccc"
    other = "dddddddd-dddd-dddd-dddd-dddddddddddd"

    assert %{"room_id" => "old-owned-priv"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "old-owned-priv", visibility: "private"})
             |> json_response(201)

    # Touch many newer public rooms so a naive ORDER BY LIMIT would drop old private.
    for i <- 1..5 do
      assert %{"room_id" => _} =
               build_conn()
               |> TestAuthHelpers.put_auth(signer, other)
               |> post("/api/v1/rooms", %{
                 room_id: "newer-pub-#{i}",
                 visibility: "public"
               })
               |> json_response(201)
    end

    # x0080: dormant owned rooms appear on mine=1, not Public projection.
    mine =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> get("/api/v1/rooms?mine=1")
      |> json_response(200)

    assert "old-owned-priv" in Enum.map(mine["rooms"], & &1["room_id"])

    activate_host!(signer, owner, "old-owned-priv")

    body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> get("/api/v1/rooms")
      |> json_response(200)

    ids = Enum.map(body["rooms"], & &1["room_id"])
    assert "old-owned-priv" in ids
  end

  test "catalog includes friends room for friend viewer and omits for stranger (x007c)", %{
    conn: conn,
    signer: signer
  } do
    owner = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
    friend = "ffffffff-ffff-ffff-ffff-ffffffffffff"
    stranger = "99999999-9999-9999-9999-999999999999"

    previous = Application.get_env(:world, :auth_access_stub)

    on_exit(fn ->
      Application.put_env(:world, :auth_access_stub, previous)
    end)

    Application.put_env(:world, :auth_access_stub, fn
      ^owner, ^friend, "friends" -> true
      ^owner, ^stranger, "friends" -> false
      _, _, _ -> false
    end)

    assert %{"room_id" => "friends-cat"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "friends-cat", visibility: "friends"})
             |> json_response(201)

    activate_host!(signer, owner, "friends-cat")

    friend_ids =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, friend)
      |> get("/api/v1/rooms")
      |> json_response(200)
      |> Map.fetch!("rooms")
      |> Enum.map(& &1["room_id"])

    assert "friends-cat" in friend_ids

    stranger_ids =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, stranger)
      |> get("/api/v1/rooms")
      |> json_response(200)
      |> Map.fetch!("rooms")
      |> Enum.map(& &1["room_id"])

    refute "friends-cat" in stranger_ids
  end

  test "rejects room create when owner cap is exceeded (x0071)", %{conn: conn, signer: signer} do
    previous = Application.get_env(:world, :max_rooms_per_owner)
    Application.put_env(:world, :max_rooms_per_owner, 1)

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, :max_rooms_per_owner)
        value -> Application.put_env(:world, :max_rooms_per_owner, value)
      end
    end)

    sub = "66666666-6666-6666-6666-666666666666"

    assert %{"room_id" => "cap-http-1"} =
             conn
             |> TestAuthHelpers.put_auth(signer, sub)
             |> post("/api/v1/rooms", %{room_id: "cap-http-1"})
             |> json_response(201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms", %{room_id: "cap-http-2"})

    assert %{"error" => "room_cap_exceeded"} = json_response(conn, 403)
  end

  test "creates room and issues host token to owner", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    conn = TestAuthHelpers.put_auth(conn, signer, sub)
    conn = post(conn, "/api/v1/rooms", %{room_id: "local"})
    assert %{"room_id" => "local", "owner_sub" => ^sub} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post(
        "/api/v1/rooms/local/tokens",
        token_params(%{role: "host", sub: "ignored-body-sub"})
      )

    body = json_response(conn, 200)
    assert is_binary(body["token"])
    assert body["room_id"] == "local"
    assert body["role"] == "host"
    assert body["sub"] == sub
    assert body["expires_in"] > 0

    assert {:ok, claims} = World.Token.verify(body["token"])
    assert is_binary(claims["cnf"]["spk"])
  end

  test "token without session_pub is rejected", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms", %{room_id: "nocnf"})

    assert %{"room_id" => "nocnf"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/nocnf/tokens", %{role: "host"})

    assert %{"error" => "invalid_session_pub"} = json_response(conn, 422)
  end

  test "token on missing room does not create or grant host", %{signer: signer} do
    guest = "33333333-3333-3333-3333-333333333333"

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/ghost/tokens", token_params(%{role: "guest"}))

    assert %{"error" => "room_not_found"} = json_response(conn, 404)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/ghost/tokens", token_params(%{role: "host"}))

    assert %{"error" => "room_not_found"} = json_response(conn, 404)
  end

  test "non-owner cannot issue host token", %{conn: conn, signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms", %{room_id: "owned"})

    assert %{"room_id" => "owned"} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/owned/tokens", token_params(%{role: "host"}))

    assert %{"error" => "host_role_forbidden"} = json_response(conn, 403)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/owned/tokens", token_params(%{role: "guest"}))

    body = json_response(conn, 200)
    assert body["role"] == "guest"
    assert body["sub"] == guest
    # x004e: wire_cap + Allocation Token for Relay Agent
    assert is_binary(body["wire_cap"]) and body["wire_cap"] != ""
    assert is_binary(body["allocation_token"]) and body["allocation_token"] != ""
    assert {:ok, alloc_claims} = World.AllocationToken.verify(body["allocation_token"])
    assert alloc_claims["aud"] == "alchemy-relay"
    assert alloc_claims["wire_cap"] == body["wire_cap"]
    assert alloc_claims["role"] == "guest"
    # x007a: room_jti correlates with Room Token jti
    assert {:ok, room_claims} = World.Token.verify(body["token"])
    assert alloc_claims["room_jti"] == room_claims["jti"]
    assert alloc_claims["jti"] != room_claims["jti"]
    # x0069: room_key is URL-safe base64 of 32 bytes; not in Allocation JWT
    assert is_binary(body["room_key"]) and body["room_key"] != ""
    assert {:ok, room_key_raw} = Base.url_decode64(body["room_key"], padding: false)
    assert byte_size(room_key_raw) == 32
    refute Map.has_key?(alloc_claims, "room_key")
    # x007e: epoch accompanies room_key (also absent from Allocation JWT)
    assert body["room_key_epoch"] == 0
    refute Map.has_key?(alloc_claims, "room_key_epoch")

    # Same room → same room_key / epoch on re-issue (no rotate)
    body2 =
      conn
      |> post("/api/v1/rooms/owned/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    assert body2["room_key"] == body["room_key"]
    assert body2["room_key_epoch"] == body["room_key_epoch"]
    # No grace yet → host Token also omits prev
    refute Map.has_key?(body2, "room_key_prev")
  end

  test "host Token carries RoomKeyGrace prev after rotate; guest does not (x007e)", %{
    conn: conn,
    signer: signer
  } do
    owner = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"

    assert %{"room_id" => "grace-tok"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "grace-tok", visibility: "public"})
             |> json_response(201)

    before =
      conn
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/grace-tok/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    assert before["room_key_epoch"] == 0
    refute Map.has_key?(before, "room_key_prev")

    assert {:ok, 1} = World.Rooms.rotate_room_key("grace-tok", owner)

    host_body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/grace-tok/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    assert host_body["room_key_epoch"] == 1
    assert host_body["room_key"] != before["room_key"]
    assert host_body["room_key_prev"] == before["room_key"]
    assert host_body["room_key_prev_epoch"] == 0

    guest_body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/grace-tok/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    assert guest_body["room_key_epoch"] == 1
    refute Map.has_key?(guest_body, "room_key_prev")
    refute Map.has_key?(guest_body, "room_key_prev_epoch")
  end

  test "csr_pem yields mtls with forced guest CN when CA configured", %{
    conn: conn,
    signer: signer
  } do
    openssl = mtls_test_openssl()
    tmp = Path.join(System.tmp_dir!(), "world-mtls-ctrl-#{System.unique_integer([:positive])}")
    File.mkdir_p!(tmp)
    on_exit(fn -> File.rm_rf(tmp) end)

    ca_key = Path.join(tmp, "ca-key.pem")
    ca_cert = Path.join(tmp, "ca-cert.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-x509",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          ca_key,
          "-out",
          ca_cert,
          "-days",
          "1",
          "-subj",
          "/CN=alchemy-test-client-ca"
        ],
        stderr_to_stdout: true
      )

    Application.put_env(:world, :mtls_ca_cert_path, ca_cert)
    Application.put_env(:world, :mtls_ca_key_path, ca_key)
    Application.put_env(:world, :mtls_work_dir, Path.join(tmp, "work"))
    Application.put_env(:world, :openssl_bin, openssl)

    on_exit(fn ->
      Application.delete_env(:world, :mtls_ca_cert_path)
      Application.delete_env(:world, :mtls_ca_key_path)
      Application.delete_env(:world, :mtls_work_dir)
      Application.delete_env(:world, :openssl_bin)
    end)

    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"

    assert %{"room_id" => "mtls-room"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "mtls-room"})
             |> json_response(201)

    key = Path.join(tmp, "client.key.pem")
    csr_path = Path.join(tmp, "client.csr.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          key,
          "-out",
          csr_path,
          "-subj",
          "/CN=alchemy-core"
        ],
        stderr_to_stdout: true
      )

    csr_pem = File.read!(csr_path)

    body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post(
        "/api/v1/rooms/mtls-room/tokens",
        token_params(%{role: "guest", csr_pem: csr_pem})
      )
      |> json_response(200)

    assert is_binary(body["allocation_token"])
    assert %{"client_cert_pem" => cert, "ca_cert_pem" => ca, "expires_at" => exp} = body["mtls"]
    assert String.contains?(cert, "BEGIN CERTIFICATE")
    assert String.contains?(ca, "BEGIN CERTIFICATE")
    assert is_binary(exp)

    issued = Path.join(tmp, "issued.pem")
    File.write!(issued, cert)

    {subject, 0} =
      System.cmd(openssl, ["x509", "-in", issued, "-noout", "-subject"], stderr_to_stdout: true)

    assert subject =~ "alchemy-guest"
    refute subject =~ "alchemy-core"
  end

  test "csr_pem without CA is ignored (token without mtls)", %{conn: conn, signer: signer} do
    Application.delete_env(:world, :mtls_ca_cert_path)
    Application.delete_env(:world, :mtls_ca_key_path)
    Application.delete_env(:world, :mtls_ca_cert_pem)
    Application.delete_env(:world, :mtls_ca_key_pem)

    owner = "22222222-2222-2222-2222-222222222222"

    assert %{"room_id" => "nocacert"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "nocacert"})
             |> json_response(201)

    body =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post(
        "/api/v1/rooms/nocacert/tokens",
        token_params(%{
          role: "guest",
          csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\nMIIB\n-----END CERTIFICATE REQUEST-----"
        })
      )
      |> json_response(200)

    assert is_binary(body["token"])
    refute Map.has_key?(body, "mtls")
  end

  test "health and jwks", %{conn: conn} do
    conn = get(conn, "/health")
    assert %{"status" => "ok"} = json_response(conn, 200)

    conn = get(build_conn(), "/.well-known/jwks.json")
    assert %{"keys" => [%{"kty" => "RSA", "kid" => _kid}]} = json_response(conn, 200)
  end

  test "conflict on duplicate room", %{conn: conn, signer: signer} do
    conn = TestAuthHelpers.put_auth(conn, signer)

    assert %{"room_id" => "dup"} =
             json_response(post(conn, "/api/v1/rooms", %{room_id: "dup"}), 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer)
      |> post("/api/v1/rooms", %{room_id: "dup"})

    assert %{"error" => "room_already_exists"} = json_response(conn, 409)
  end

  test "friends visibility forbids non-friend guest", %{conn: conn, signer: signer} do
    owner = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    guest = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

    previous = Application.get_env(:world, :auth_access_stub)

    on_exit(fn ->
      Application.put_env(:world, :auth_access_stub, previous)
    end)

    Application.put_env(:world, :auth_access_stub, fn ^owner, ^guest, "friends" -> false end)

    assert %{"visibility" => "friends"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "friends-room", visibility: "friends"})
             |> json_response(201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/friends-room/tokens", token_params(%{role: "guest"}))

    assert %{"error" => "visibility_forbidden"} = json_response(conn, 403)
  end

  test "private visibility allows owner host token only", %{conn: conn, signer: signer} do
    owner = "cccccccc-cccc-cccc-cccc-cccccccccccc"
    guest = "dddddddd-dddd-dddd-dddd-dddddddddddd"

    assert %{"visibility" => "private"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "priv", visibility: "private"})
             |> json_response(201)

    assert %{"role" => "host"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms/priv/tokens", token_params(%{role: "host"}))
             |> json_response(200)

    assert %{"error" => "visibility_forbidden"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, guest)
             |> post("/api/v1/rooms/priv/tokens", token_params(%{role: "guest"}))
             |> json_response(403)
  end

  test "lan visibility rejects cf-connecting-ip path", %{conn: conn, signer: signer} do
    owner = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
    guest = "ffffffff-ffff-ffff-ffff-ffffffffffff"

    assert %{"visibility" => "lan"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "lan1", visibility: "lan"})
             |> json_response(201)

    assert %{"error" => "lan_required"} =
             build_conn()
             |> Map.put(:remote_ip, {192, 168, 0, 5})
             |> Plug.Conn.put_req_header("cf-connecting-ip", "203.0.113.1")
             |> TestAuthHelpers.put_auth(signer, guest)
             |> post("/api/v1/rooms/lan1/tokens", token_params(%{role: "guest"}))
             |> json_response(403)
  end

  test "host_keys issues attestation for owner host token", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms", %{room_id: "attest"})

    assert %{"room_id" => "attest"} = json_response(conn, 201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/attest/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    conn =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/attest/host_keys", %{host_pub: host_pub})

    body = json_response(conn, 200)
    assert is_binary(body["attestation"])
    assert body["expires_in"] > 0
    assert body["room_id"] == "attest"
    assert body["host_generation"] == 1

    # Same pub → same generation; different pub → bump (x003a).
    body2 =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/attest/host_keys", %{host_pub: host_pub})
      |> json_response(200)

    assert body2["host_generation"] == 1

    other_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)

    body3 =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/attest/host_keys", %{host_pub: other_pub})
      |> json_response(200)

    assert body3["host_generation"] == 2
  end

  test "host_keys rejects guest token", %{conn: conn, signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"
    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "guest-deny"})
    |> json_response(201)

    %{"token" => guest_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/guest-deny/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    conn =
      build_conn()
      |> put_req_header("authorization", "Bearer #{guest_token}")
      |> post("/api/v1/rooms/guest-deny/host_keys", %{host_pub: host_pub})

    assert %{"error" => "host_role_required"} = json_response(conn, 403)
  end

  test "host_keys rejects missing bearer and bad host_pub", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"

    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> post("/api/v1/rooms", %{room_id: "badpub"})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/badpub/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    assert %{"error" => "missing_room_token"} =
             build_conn()
             |> post("/api/v1/rooms/badpub/host_keys", %{host_pub: "AA"})
             |> json_response(401)

    assert %{"error" => "invalid_host_pub"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/badpub/host_keys", %{host_pub: "not-base64!!!"})
             |> json_response(422)
  end

  test "scene returns demo definition for owner host token", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"

    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> post("/api/v1/rooms", %{room_id: "scene-room"})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/scene-room/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    body =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> get("/api/v1/rooms/scene-room/scene")
      |> json_response(200)

    assert body["room_id"] == "scene-room"
    assert body["revision"] == 1
    assert is_list(body["entries"])
    assert length(body["entries"]) == 14
    assert body["locomotion"]["mode"] == "noclip"
  end

  test "scene rejects guest token", %{conn: conn, signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "scene-guest"})
    |> json_response(201)

    %{"token" => guest_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/scene-guest/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    assert %{"error" => "host_role_required"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{guest_token}")
             |> get("/api/v1/rooms/scene-guest/scene")
             |> json_response(403)
  end

  test "scene returns 404 for unknown room", %{signer: signer} do
    _ = signer
    owner = "22222222-2222-2222-2222-222222222222"
    # Token.generate does not require the room registry entry (API create_token does).
    assert {:ok, ghost_token, _jti, _ttl} =
             World.Token.generate("ghost-scene", "host", owner, sample_session_pub_bytes())

    assert %{"error" => "room_not_found"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{ghost_token}")
             |> get("/api/v1/rooms/ghost-scene/scene")
             |> json_response(404)
  end

  test "scene rejects missing bearer", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"

    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> post("/api/v1/rooms", %{room_id: "scene-nobearer"})
    |> json_response(201)

    assert %{"error" => "missing_room_token"} =
             build_conn()
             |> get("/api/v1/rooms/scene-nobearer/scene")
             |> json_response(401)
  end

  defp host_pub_b64 do
    Base.url_encode64(:crypto.strong_rand_bytes(32), padding: false)
  end

  defp issue_attestation(host_token, room_id, host_pub_b64) do
    %{"attestation" => attestation} =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/#{room_id}/host_keys", %{host_pub: host_pub_b64})
      |> json_response(200)

    attestation
  end

  test "manifest_seal issues seal for canonical scene hash", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"

    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> post("/api/v1/rooms", %{room_id: "seal"})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/seal/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    attestation = issue_attestation(host_token, "seal", host_pub_b64())

    assert {:ok, revision, host_generation, blake3} =
             World.CanonicalManifest.canonical_blake3("seal", attestation)

    body =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/seal/manifest_seal", %{
        revision: revision,
        manifest_blake3: blake3,
        host_attestation: attestation
      })
      |> json_response(200)

    assert is_binary(body["seal"])
    assert body["expires_in"] > 0
    assert body["room_id"] == "seal"
    assert body["manifest_blake3"] == blake3
    assert body["revision"] == revision
    assert body["host_generation"] == host_generation
    assert host_generation == 1
  end

  test "manifest_seal rejects arbitrary blake3 oracle (x0039)", %{conn: conn, signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    oracle = String.duplicate("cd", 32)

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "seal-oracle"})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/seal-oracle/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    attestation = issue_attestation(host_token, "seal-oracle", host_pub_b64())

    assert %{"error" => "manifest_blake3_mismatch"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/seal-oracle/manifest_seal", %{
               revision: 1,
               manifest_blake3: oracle,
               host_attestation: attestation
             })
             |> json_response(422)
  end

  test "manifest_seal rejects revision_mismatch and invalid_host_attestation", %{
    conn: conn,
    signer: signer
  } do
    owner = "22222222-2222-2222-2222-222222222222"

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "seal-rev"})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/seal-rev/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    attestation = issue_attestation(host_token, "seal-rev", host_pub_b64())

    assert {:ok, revision, _gen, blake3} =
             World.CanonicalManifest.canonical_blake3("seal-rev", attestation)

    assert %{"error" => "revision_mismatch"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/seal-rev/manifest_seal", %{
               revision: revision + 1,
               manifest_blake3: blake3,
               host_attestation: attestation
             })
             |> json_response(422)

    assert %{"error" => "invalid_host_attestation"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/seal-rev/manifest_seal", %{
               revision: revision,
               manifest_blake3: blake3,
               host_attestation: "not.a.jwt"
             })
             |> json_response(422)
  end

  test "manifest_seal rejects guest token and bad blake3", %{conn: conn, signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"
    blake3 = String.duplicate("ef", 32)

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "seal-deny"})
    |> json_response(201)

    %{"token" => guest_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/seal-deny/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    assert %{"error" => "host_role_required"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{guest_token}")
             |> post("/api/v1/rooms/seal-deny/manifest_seal", %{
               revision: 1,
               manifest_blake3: blake3,
               host_attestation: "eyJ.fake"
             })
             |> json_response(403)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/seal-deny/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    attestation = issue_attestation(host_token, "seal-deny", host_pub_b64())

    assert %{"error" => "invalid_manifest_blake3"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/seal-deny/manifest_seal", %{
               revision: 1,
               manifest_blake3: "nope",
               host_attestation: attestation
             })
             |> json_response(422)

    assert %{"error" => "missing_host_attestation"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/seal-deny/manifest_seal", %{
               revision: 1,
               manifest_blake3: blake3
             })
             |> json_response(422)
  end

  test "manifest_seal rejects room_id mismatch and missing room", %{signer: signer} do
    owner = "22222222-2222-2222-2222-222222222222"
    blake3 = String.duplicate("aa", 32)

    build_conn()
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "seal-a"})
    |> json_response(201)

    build_conn()
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: "seal-b"})
    |> json_response(201)

    %{"token" => token_a} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/seal-a/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    attestation = issue_attestation(token_a, "seal-a", host_pub_b64())

    assert %{"error" => "room_id_mismatch"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{token_a}")
             |> post("/api/v1/rooms/seal-b/manifest_seal", %{
               revision: 1,
               manifest_blake3: blake3,
               host_attestation: attestation
             })
             |> json_response(403)

    # Token.generate does not require the room registry entry (API create_token does).
    assert {:ok, ghost_token, _jti, _ttl} =
             World.Token.generate("ghost-seal", "host", owner, sample_session_pub_bytes())

    assert %{"error" => "room_not_found"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{ghost_token}")
             |> post("/api/v1/rooms/ghost-seal/manifest_seal", %{
               revision: 1,
               manifest_blake3: blake3,
               host_attestation: attestation
             })
             |> json_response(404)
  end

  test "create returns 503 storage_unavailable when repo is down", %{signer: signer} do
    stop_repo!()

    assert %{"error" => "storage_unavailable"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer)
             |> post("/api/v1/rooms", %{room_id: "down-room"})
             |> json_response(503)
  after
    restart_repo!()
  end

  defp stop_repo! do
    case Supervisor.terminate_child(World.Supervisor, World.Repo) do
      :ok -> :ok
      {:error, :not_found} -> :ok
    end
  end

  defp mtls_test_openssl do
    found =
      [
        System.get_env("OPENSSL_BIN"),
        "openssl",
        "C:/Program Files/Git/usr/bin/openssl.exe",
        "C:/Program Files/OpenSSL-Win64/bin/openssl.exe"
      ]
      |> Enum.reject(&is_nil/1)
      |> Enum.find(fn bin ->
        try do
          match?({_, 0}, System.cmd(bin, ["version"], stderr_to_stdout: true))
        rescue
          _ -> false
        end
      end)

    found || flunk("openssl not found (set OPENSSL_BIN)")
  end

  defp restart_repo! do
    case Supervisor.restart_child(World.Supervisor, World.Repo) do
      {:ok, _} -> :ok
      {:ok, _, _} -> :ok
      {:error, {:already_started, _}} -> :ok
      {:error, :running} -> :ok
      other -> flunk("failed to restart World.Repo: #{inspect(other)}")
    end

    # x003b: Repo restart leaves Sandbox in auto; restore manual for later DataCase owners.
    Sandbox.mode(World.Repo, :manual)
  end

  defp issue_seal(host_token, room_id) do
    attestation = issue_attestation(host_token, room_id, host_pub_b64())

    assert {:ok, revision, host_generation, blake3} =
             World.CanonicalManifest.canonical_blake3(room_id, attestation)

    body =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/#{room_id}/manifest_seal", %{
        revision: revision,
        manifest_blake3: blake3,
        host_attestation: attestation
      })
      |> json_response(200)

    {body["seal"], revision, host_generation, blake3}
  end

  test "place_read issues JWT for guest with verified world_seal (x006b)", %{
    conn: conn,
    signer: signer
  } do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"
    room = "place-read"

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: room})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/#{room}/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    {seal, revision, host_generation, manifest_blake3} = issue_seal(host_token, room)

    %{"token" => guest_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/#{room}/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    body =
      build_conn()
      |> put_req_header("authorization", "Bearer #{guest_token}")
      |> post("/api/v1/rooms/#{room}/place_read", %{world_seal: seal})
      |> json_response(200)

    assert is_binary(body["place_read"])
    assert body["expires_in"] > 0
    assert body["revision"] == revision
    assert body["host_generation"] == host_generation
    assert body["manifest_blake3"] == manifest_blake3
    assert body["blake3_count"] >= 1
    assert body["extra_blake3"] == []

    assert {:ok, claims} = World.PlaceRead.verify(body["place_read"])
    assert claims["sub"] == guest
    assert claims["typ"] == "place_read"
    assert claims["room_id"] == room

    coin = World.DemoScene.build()["entries"] |> Enum.find(&(&1["blake3"] != ""))

    assert coin["blake3"] in (World.PlaceHashes.resolve(room, revision, manifest_blake3)
                              |> elem(1))

    hashes =
      build_conn()
      |> put_req_header("authorization", "Bearer #{body["place_read"]}")
      |> get("/api/v1/rooms/#{room}/place_hashes")
      |> json_response(200)

    assert coin["blake3"] in hashes["blake3s"]
    assert hashes["manifest_blake3"] == manifest_blake3
  end

  test "place_read rejects missing seal, forged seal, wrong-room seal, extra_blake3 (x006b)", %{
    conn: conn,
    signer: signer
  } do
    owner = "22222222-2222-2222-2222-222222222222"
    room = "place-read-bad"
    other = "place-read-other"

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: room})
    |> json_response(201)

    build_conn()
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: other})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/#{room}/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    %{"token" => other_host} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/#{other}/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    {seal, _rev, _gen, _blake3} = issue_seal(host_token, room)
    {other_seal, _, _, _} = issue_seal(other_host, other)

    assert %{"error" => "missing_world_seal"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/#{room}/place_read", %{})
             |> json_response(422)

    assert %{"error" => "invalid_world_seal"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/#{room}/place_read", %{
               world_seal: seal <> "tampered"
             })
             |> json_response(422)

    assert %{"error" => "seal_room_mismatch"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/#{room}/place_read", %{world_seal: other_seal})
             |> json_response(403)

    one_extra = [String.duplicate("ab", 32)]

    assert %{"error" => "extra_blake3_disabled"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> post("/api/v1/rooms/#{room}/place_read", %{
               world_seal: seal,
               extra_blake3: one_extra
             })
             |> json_response(422)
  end

  # x007b: DELETE …/scene/entries/:entity_id HTTP contract
  test "delete_scene_entry removes mesh for host and maps guest/invalid/not_mesh", %{
    conn: conn,
    signer: signer
  } do
    owner = "22222222-2222-2222-2222-222222222222"
    guest = "33333333-3333-3333-3333-333333333333"
    room = "scene-del-http"
    blake3 = String.duplicate("cd", 32)

    conn
    |> TestAuthHelpers.put_auth(signer, owner)
    |> post("/api/v1/rooms", %{room_id: room})
    |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> post("/api/v1/rooms/#{room}/tokens", token_params(%{role: "host"}))
      |> json_response(200)

    %{"token" => guest_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/#{room}/tokens", token_params(%{role: "guest"}))
      |> json_response(200)

    scene =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/#{room}/scene/entries", %{blake3: blake3, x: 1.0, y: 1.0, z: 1.0})
      |> json_response(200)

    entity_id = List.last(scene["entries"])["entity_id"]
    assert is_integer(entity_id)

    box_id =
      scene["entries"]
      |> Enum.find(fn e ->
        (e["blake3"] == "" or e["blake3"] == nil) and (e["half_extent"] || 0) > 0
      end)
      |> Map.fetch!("entity_id")

    assert %{"error" => "host_role_required"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{guest_token}")
             |> delete("/api/v1/rooms/#{room}/scene/entries/#{entity_id}")
             |> json_response(403)

    assert %{"error" => "invalid_entity_id"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> delete("/api/v1/rooms/#{room}/scene/entries/not-an-id")
             |> json_response(422)

    assert %{"error" => "not_mesh_entry"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> delete("/api/v1/rooms/#{room}/scene/entries/#{box_id}")
             |> json_response(422)

    assert %{"error" => "entry_not_found"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{host_token}")
             |> delete("/api/v1/rooms/#{room}/scene/entries/999999")
             |> json_response(404)

    deleted =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> delete("/api/v1/rooms/#{room}/scene/entries/#{entity_id}")
      |> json_response(200)

    assert deleted["room_id"] == room
    refute Enum.any?(deleted["entries"], fn e -> e["entity_id"] == entity_id end)
  end

  test "handover nominates guest host without changing owner (x0080)", %{
    conn: conn,
    signer: signer
  } do
    owner = "10101010-1010-1010-1010-101010101010"
    guest = "20202020-2020-2020-2020-202020202020"

    assert %{"room_id" => "ho-1"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "ho-1", visibility: "public"})
             |> json_response(201)

    activate_host!(signer, owner, "ho-1")

    assert %{"state" => "active", "active_host_sub" => ^guest, "owner_sub" => ^owner} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms/ho-1/handover", %{target_sub: guest})
             |> json_response(200)

    # Nominated guest may issue host Token; previous host may not.
    assert %{"role" => "host", "active_host_sub" => ^guest} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, guest)
             |> post("/api/v1/rooms/ho-1/tokens", token_params(%{role: "host"}))
             |> json_response(200)

    assert %{"error" => "host_role_forbidden"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms/ho-1/tokens", token_params(%{role: "host"}))
             |> json_response(403)
  end

  test "handover null dormants session and drops public catalog (x0080)", %{
    conn: conn,
    signer: signer
  } do
    owner = "30303030-3030-3030-3030-303030303030"
    guest = "40404040-4040-4040-4040-404040404040"

    assert %{"room_id" => "ho-dorm"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "ho-dorm", visibility: "public"})
             |> json_response(201)

    activate_host!(signer, owner, "ho-dorm")

    assert %{"state" => "dormant", "active_host_sub" => nil} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms/ho-dorm/handover", %{target_sub: nil})
             |> json_response(200)

    public_ids =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> get("/api/v1/rooms")
      |> json_response(200)
      |> Map.fetch!("rooms")
      |> Enum.map(& &1["room_id"])

    refute "ho-dorm" in public_ids

    mine =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, owner)
      |> get("/api/v1/rooms?mine=1")
      |> json_response(200)
      |> Map.fetch!("rooms")
      |> Enum.find(&(&1["room_id"] == "ho-dorm"))

    assert mine["state"] == "dormant"
  end

  test "takeover CAS succeeds once from dormant (x0080)", %{conn: conn, signer: signer} do
    owner = "50505050-5050-5050-5050-505050505050"
    guest_a = "60606060-6060-6060-6060-606060606060"
    guest_b = "70707070-7070-7070-7070-707070707070"

    assert %{"room_id" => "to-1"} =
             conn
             |> TestAuthHelpers.put_auth(signer, owner)
             |> post("/api/v1/rooms", %{room_id: "to-1", visibility: "public"})
             |> json_response(201)

    assert %{"state" => "active", "active_host_sub" => ^guest_a} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, guest_a)
             |> post("/api/v1/rooms/to-1/takeover", %{})
             |> json_response(200)

    assert %{"error" => "takeover_conflict"} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, guest_b)
             |> post("/api/v1/rooms/to-1/takeover", %{})
             |> json_response(409)
  end
end
