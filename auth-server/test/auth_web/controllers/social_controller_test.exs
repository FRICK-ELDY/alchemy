defmodule AuthWeb.SocialControllerTest do
  use AuthWeb.ConnCase, async: true

  import Auth.AccountsFixtures

  alias Auth.{Accounts, Social}

  setup do
    alice = user_fixture(%{username: "alice_api"})
    bob = user_fixture(%{username: "bob_api"})
    {:ok, alice_session} = Accounts.issue_session(alice, false)
    {:ok, bob_session} = Accounts.issue_session(bob, false)

    %{
      alice: alice,
      bob: bob,
      alice_token: alice_session.access_token,
      bob_token: bob_session.access_token
    }
  end

  test "follow then friend request without mutual returns 422", %{
    alice_token: token,
    bob: bob
  } do
    conn =
      build_conn()
      |> put_req_header("authorization", "Bearer #{token}")
      |> post("/api/v1/social/follows/#{bob.id}")

    assert %{"ok" => true} = json_response(conn, 200)

    conn =
      build_conn()
      |> put_req_header("authorization", "Bearer #{token}")
      |> post("/api/v1/social/friend-requests/#{bob.id}")

    assert %{"error" => "not_mutual"} = json_response(conn, 422)
  end

  test "mutual follow then accept friend", %{
    alice: alice,
    bob: bob,
    alice_token: alice_token,
    bob_token: bob_token
  } do
    assert %{"ok" => true} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{alice_token}")
             |> post("/api/v1/social/follows/#{bob.id}")
             |> json_response(200)

    assert %{"ok" => true} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{bob_token}")
             |> post("/api/v1/social/follows/#{alice.id}")
             |> json_response(200)

    assert %{"ok" => true, "state" => "pending"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{alice_token}")
             |> post("/api/v1/social/friend-requests/#{bob.id}")
             |> json_response(201)

    assert %{"ok" => true, "state" => "accepted"} =
             build_conn()
             |> put_req_header("authorization", "Bearer #{bob_token}")
             |> post("/api/v1/social/friend-requests/#{alice.id}/accept")
             |> json_response(200)

    assert Social.friends?(alice.id, bob.id)
  end

  test "internal access requires token", %{alice: alice, bob: bob} do
    conn =
      get(build_conn(), "/internal/v1/access", %{
        owner_sub: alice.id,
        viewer_sub: bob.id,
        visibility: "public"
      })

    assert %{"error" => "missing_internal_token"} = json_response(conn, 401)

    conn =
      build_conn()
      |> put_req_header("x-alchemy-internal-token", "dev-internal-token")
      |> get("/internal/v1/access", %{
        owner_sub: alice.id,
        viewer_sub: bob.id,
        visibility: "public"
      })

    assert %{"allowed" => true} = json_response(conn, 200)
  end
end
