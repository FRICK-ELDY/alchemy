defmodule WorldWeb.RoomControllerTest do
  use WorldWeb.ConnCase, async: false

  test "creates room and issues token", %{conn: conn} do
    conn = post(conn, "/api/v1/rooms", %{room_id: "local"})
    assert %{"room_id" => "local"} = json_response(conn, 201)

    conn = post(build_conn(), "/api/v1/rooms/local/tokens", %{role: "host", sub: "dev"})
    body = json_response(conn, 200)
    assert is_binary(body["token"])
    assert body["room_id"] == "local"
    assert body["role"] == "host"
    assert body["expires_in"] > 0
  end

  test "health and jwks", %{conn: conn} do
    conn = get(conn, "/health")
    assert %{"status" => "ok"} = json_response(conn, 200)

    conn = get(build_conn(), "/.well-known/jwks.json")
    assert %{"keys" => [%{"kty" => "RSA", "kid" => _kid}]} = json_response(conn, 200)
  end

  test "conflict on duplicate room", %{conn: conn} do
    assert %{"room_id" => "dup"} =
             json_response(post(conn, "/api/v1/rooms", %{room_id: "dup"}), 201)

    conn = post(build_conn(), "/api/v1/rooms", %{room_id: "dup"})
    assert %{"error" => "room_already_exists"} = json_response(conn, 409)
  end
end
