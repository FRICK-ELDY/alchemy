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

  test "creates room and issues token with auth JWT", %{conn: conn, signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    conn = TestAuthHelpers.put_auth(conn, signer, sub)
    conn = post(conn, "/api/v1/rooms", %{room_id: "local"})
    assert %{"room_id" => "local"} = json_response(conn, 201)

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
end
