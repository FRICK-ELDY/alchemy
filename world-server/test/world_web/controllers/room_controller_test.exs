defmodule WorldWeb.RoomControllerTest do
  use WorldWeb.ConnCase, async: false

  alias World.TestAuthHelpers

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer}
  end

  test "rejects room create without auth", %{conn: conn} do
    conn = post(conn, "/api/v1/rooms", %{room_id: "noauth"})
    assert %{"error" => "missing_auth_token"} = json_response(conn, 401)
  end

  test "creates room and issues host token to owner", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    conn = TestAuthHelpers.put_auth(conn, signer, sub)
    conn = post(conn, "/api/v1/rooms", %{room_id: "local"})
    assert %{"room_id" => "local", "owner_sub" => ^sub} = json_response(conn, 201)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/local/tokens", %{role: "host", sub: "ignored-body-sub"})

    body = json_response(conn, 200)
    assert is_binary(body["token"])
    assert body["room_id"] == "local"
    assert body["role"] == "host"
    assert body["sub"] == sub
    assert body["expires_in"] > 0
  end

  test "token on missing room does not create or grant host", %{signer: signer} do
    guest = "33333333-3333-3333-3333-333333333333"

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/ghost/tokens", %{role: "guest"})

    assert %{"error" => "room_not_found"} = json_response(conn, 404)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/ghost/tokens", %{role: "host"})

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
      |> post("/api/v1/rooms/owned/tokens", %{role: "host"})

    assert %{"error" => "host_role_owner_only"} = json_response(conn, 403)

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, guest)
      |> post("/api/v1/rooms/owned/tokens", %{role: "guest"})

    body = json_response(conn, 200)
    assert body["role"] == "guest"
    assert body["sub"] == guest
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
      |> post("/api/v1/rooms/attest/tokens", %{role: "host"})
      |> json_response(200)

    conn =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/attest/host_keys", %{host_pub: host_pub})

    body = json_response(conn, 200)
    assert is_binary(body["attestation"])
    assert body["expires_in"] > 0
    assert body["room_id"] == "attest"
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
      |> post("/api/v1/rooms/guest-deny/tokens", %{role: "guest"})
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
      |> post("/api/v1/rooms/badpub/tokens", %{role: "host"})
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
end
