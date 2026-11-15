defmodule AssetsWeb.SpawnTicketControllerTest do
  use AssetsWeb.ConnCase, async: false, cas: true

  alias Assets.Blobs
  alias Assets.TestAuthHelpers

  @sub "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    %{signer: signer}
  end

  test "jwks has no private material", %{conn: conn} do
    conn = get(conn, "/.well-known/jwks.json")
    body = json_response(conn, 200)
    assert %{"keys" => [key]} = body
    assert key["kty"] == "RSA"
    assert is_binary(key["kid"])
    refute Map.has_key?(key, "d")
    refute Map.has_key?(key, "p")
    refute Map.has_key?(key, "q")
  end

  test "create issues ticket when Grant exists", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub, "issue-mesh", "application/octet-stream")

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/spawn_tickets",
        Jason.encode!(%{room_id: "local", blake3s: [cas_obj.blake3]})
      )

    assert %{"spawn_ticket" => ticket, "expires_in" => ttl} = json_response(conn, 200)
    assert is_binary(ticket) and ticket != ""
    assert ttl == 60
  end

  test "create without Grant is forbidden", %{conn: conn, signer: signer} do
    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/spawn_tickets",
        Jason.encode!(%{room_id: "local", blake3s: [String.duplicate("11", 32)]})
      )

    assert %{"error" => "no_grant"} = json_response(conn, 403)
  end

  test "take_out with room_id bundles spawn_ticket", %{conn: conn, signer: signer} do
    {:ok, cas_obj, _} = Blobs.upload(@sub, "take-out-ticket", "application/octet-stream")
    {:ok, node} = Assets.Drive.create_blob(@sub, "mesh", cas_obj.blake3)

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, @sub)
      |> put_req_header("content-type", "application/json")
      |> post(
        "/api/v1/drive/take_out",
        Jason.encode!(%{node_id: node.node_id, room_id: "local"})
      )

    assert %{
             "blobs" => [%{"blake3" => b3}],
             "spawn_ticket" => ticket,
             "expires_in" => _
           } = json_response(conn, 200)

    assert b3 == cas_obj.blake3
    assert is_binary(ticket) and ticket != ""
  end
end
