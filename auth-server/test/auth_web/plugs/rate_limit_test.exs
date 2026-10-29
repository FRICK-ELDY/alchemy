defmodule AuthWeb.Plugs.RateLimitTest do
  use AuthWeb.ConnCase, async: false

  import Auth.AccountsFixtures

  alias Auth.RateLimit

  @strict_limits %{
    login: %{
      ip: %{limit: 2, period_ms: 60_000},
      identifier: %{limit: 2, period_ms: 60_000}
    },
    register: %{
      ip: %{limit: 2, period_ms: 60_000},
      email: %{limit: 2, period_ms: 60_000}
    },
    refresh: %{
      ip: %{limit: 2, period_ms: 60_000},
      token: %{limit: 2, period_ms: 60_000}
    },
    social_mutate: %{
      ip: %{limit: 2, period_ms: 60_000},
      user: %{limit: 2, period_ms: 60_000}
    }
  }

  setup do
    RateLimit.reset()
    previous = Application.get_env(:auth, RateLimit)
    Application.put_env(:auth, RateLimit, Keyword.put(previous, :limits, @strict_limits))

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:auth, RateLimit)
        value -> Application.put_env(:auth, RateLimit, value)
      end
    end)

    :ok
  end

  defp with_ip(conn, ip) do
    conn
    |> Plug.Conn.put_req_header("x-forwarded-for", ip)
  end

  describe "POST /api/v1/auth/login" do
    setup do
      user_fixture(%{username: "rate_user", email: "rate@example.com"})
      :ok
    end

    test "returns 429 when the IP limit is exceeded", %{conn: conn} do
      conn = with_ip(conn, "10.20.30.40")

      for _ <- 1..2 do
        conn =
          post(conn, ~p"/api/v1/auth/login", %{
            "identifier" => "rate@example.com",
            "password" => "Wrong-password1"
          })

        assert json_response(conn, 401)
      end

      conn =
        post(conn, ~p"/api/v1/auth/login", %{
          "identifier" => "rate@example.com",
          "password" => "Wrong-password1"
        })

      assert %{"errors" => %{"detail" => "Too many requests"}} = json_response(conn, 429)

      [retry_after] = get_resp_header(conn, "retry-after")
      assert String.to_integer(retry_after) in 1..60
    end

    test "returns 429 when the identifier limit is exceeded", %{conn: conn} do
      for index <- 1..2 do
        conn =
          conn
          |> with_ip("10.0.0.#{index}")
          |> post(~p"/api/v1/auth/login", %{
            "identifier" => "rate@example.com",
            "password" => "Wrong-password1"
          })

        assert json_response(conn, 401)
      end

      conn =
        conn
        |> with_ip("10.0.0.99")
        |> post(~p"/api/v1/auth/login", %{
          "identifier" => "rate@example.com",
          "password" => "Wrong-password1"
        })

      assert json_response(conn, 429)
    end

    test "allows successful login below the limit", %{conn: conn} do
      conn =
        with_ip(conn, "10.20.30.50")
        |> post(~p"/api/v1/auth/login", %{
          "identifier" => "rate@example.com",
          "password" => default_password()
        })

      assert %{"access_token" => _} = json_response(conn, 200)
    end

    test "falls back to remote_ip when x-forwarded-for is blank", %{conn: conn} do
      exhausted_conn =
        conn
        |> Map.put(:remote_ip, {10, 0, 0, 1})
        |> Plug.Conn.delete_req_header("x-forwarded-for")

      for index <- 1..2 do
        exhausted_conn =
          post(exhausted_conn, ~p"/api/v1/auth/login", %{
            "identifier" => "blank-ip-#{index}@example.com",
            "password" => "Wrong-password1"
          })

        assert json_response(exhausted_conn, 401)
      end

      assert json_response(
               post(exhausted_conn, ~p"/api/v1/auth/login", %{
                 "identifier" => "blank-ip-3@example.com",
                 "password" => "Wrong-password1"
               }),
               429
             )

      other_conn =
        conn
        |> Map.put(:remote_ip, {10, 0, 0, 2})
        |> Plug.Conn.delete_req_header("x-forwarded-for")
        |> post(~p"/api/v1/auth/login", %{
          "identifier" => "other-remote@example.com",
          "password" => "Wrong-password1"
        })

      assert json_response(other_conn, 401)
    end

    test "ignores spoofed x-forwarded-for from untrusted peers", %{conn: conn} do
      attacker_ip = {203, 0, 113, 10}

      for index <- 1..2 do
        conn =
          conn
          |> Map.put(:remote_ip, attacker_ip)
          |> Plug.Conn.put_req_header("x-forwarded-for", "1.1.1.#{index}")
          |> post(~p"/api/v1/auth/login", %{
            "identifier" => "spoof-#{index}@example.com",
            "password" => "Wrong-password1"
          })

        assert json_response(conn, 401)
      end

      conn =
        conn
        |> Map.put(:remote_ip, attacker_ip)
        |> Plug.Conn.put_req_header("x-forwarded-for", "8.8.8.8")
        |> post(~p"/api/v1/auth/login", %{
          "identifier" => "spoof-3@example.com",
          "password" => "Wrong-password1"
        })

      assert json_response(conn, 429)
    end
  end

  describe "POST /api/v1/auth/register" do
    test "returns 429 when the email limit is exceeded", %{conn: conn} do
      for index <- 1..2 do
        conn =
          conn
          |> with_ip("10.1.0.#{index}")
          |> post(~p"/api/v1/auth/register", %{
            "username" => "user#{index}",
            "email" => "same@example.com",
            "password" => default_password(),
            "birthday" => "2000-01-31",
            "tos_agreed" => true
          })

        assert conn.status in [201, 422]
      end

      conn =
        conn
        |> with_ip("10.1.0.99")
        |> post(~p"/api/v1/auth/register", %{
          "username" => "another",
          "email" => "same@example.com",
          "password" => default_password(),
          "birthday" => "2000-01-31",
          "tos_agreed" => true
        })

      assert json_response(conn, 429)
    end
  end

  describe "POST /api/v1/auth/refresh" do
    setup do
      user = user_fixture()
      {:ok, session} = Auth.Accounts.issue_session(user, true)
      %{refresh_token: session.refresh_token}
    end

    test "returns 429 when the token family limit is exceeded", %{refresh_token: refresh_token} do
      current_token =
        Enum.reduce(1..2, refresh_token, fn index, token ->
          conn =
            build_conn()
            |> with_ip("10.2.0.#{index}")
            |> post(~p"/api/v1/auth/refresh", %{"refresh_token" => token})

          assert %{"refresh_token" => next_token} = json_response(conn, 200)
          next_token
        end)

      conn =
        build_conn()
        |> with_ip("10.2.0.99")
        |> post(~p"/api/v1/auth/refresh", %{"refresh_token" => current_token})

      assert json_response(conn, 429)
    end
  end

  describe "POST /api/v1/social/follows/:user_id (x0043)" do
    defp put_social_limits(limits) do
      previous = Application.get_env(:auth, RateLimit)
      Application.put_env(:auth, RateLimit, Keyword.put(previous, :limits, limits))
      RateLimit.reset()

      on_exit(fn ->
        case previous do
          nil -> Application.delete_env(:auth, RateLimit)
          value -> Application.put_env(:auth, RateLimit, value)
        end
      end)
    end

    defp social_base_limits(social_mutate) do
      %{
        login: %{
          ip: %{limit: 10_000, period_ms: 60_000},
          identifier: %{limit: 10_000, period_ms: 60_000}
        },
        register: %{
          ip: %{limit: 10_000, period_ms: 60_000},
          email: %{limit: 10_000, period_ms: 60_000}
        },
        refresh: %{
          ip: %{limit: 10_000, period_ms: 60_000},
          token: %{limit: 10_000, period_ms: 60_000}
        },
        social_mutate: social_mutate
      }
    end

    setup do
      alice = user_fixture(%{username: "social_rate_alice"})
      bob = user_fixture(%{username: "social_rate_bob"})
      carol = user_fixture(%{username: "social_rate_carol"})
      dave = user_fixture(%{username: "social_rate_dave"})
      eve = user_fixture(%{username: "social_rate_eve"})
      {:ok, alice_session} = Auth.Accounts.issue_session(alice, false)
      {:ok, bob_session} = Auth.Accounts.issue_session(bob, false)

      %{
        alice_token: alice_session.access_token,
        bob_token: bob_session.access_token,
        targets: [carol, dave, eve]
      }
    end

    test "returns 429 when the user limit is exceeded", %{
      conn: conn,
      alice_token: token,
      targets: [carol, dave, eve]
    } do
      put_social_limits(
        social_base_limits(%{
          ip: %{limit: 10_000, period_ms: 60_000},
          user: %{limit: 2, period_ms: 60_000}
        })
      )

      for target <- [carol, dave] do
        conn =
          conn
          |> with_ip("10.3.0.1")
          |> put_req_header("authorization", "Bearer #{token}")
          |> post("/api/v1/social/follows/#{target.id}")

        assert %{"ok" => true} = json_response(conn, 200)
      end

      conn =
        conn
        |> with_ip("10.3.0.1")
        |> put_req_header("authorization", "Bearer #{token}")
        |> post("/api/v1/social/follows/#{eve.id}")

      assert %{"errors" => %{"detail" => "Too many requests"}} = json_response(conn, 429)
      [retry_after] = get_resp_header(conn, "retry-after")
      assert String.to_integer(retry_after) in 1..60
    end

    test "returns 429 when the IP limit is exceeded across users", %{
      conn: conn,
      alice_token: alice_token,
      bob_token: bob_token,
      targets: [carol, dave, eve]
    } do
      # NAT-shared IP: different Bearer users still share the IP bucket.
      put_social_limits(
        social_base_limits(%{
          ip: %{limit: 2, period_ms: 60_000},
          user: %{limit: 10_000, period_ms: 60_000}
        })
      )

      assert %{"ok" => true} =
               conn
               |> with_ip("10.3.0.50")
               |> put_req_header("authorization", "Bearer #{alice_token}")
               |> post("/api/v1/social/follows/#{carol.id}")
               |> json_response(200)

      assert %{"ok" => true} =
               conn
               |> with_ip("10.3.0.50")
               |> put_req_header("authorization", "Bearer #{bob_token}")
               |> post("/api/v1/social/follows/#{dave.id}")
               |> json_response(200)

      conn =
        conn
        |> with_ip("10.3.0.50")
        |> put_req_header("authorization", "Bearer #{alice_token}")
        |> post("/api/v1/social/follows/#{eve.id}")

      assert %{"errors" => %{"detail" => "Too many requests"}} = json_response(conn, 429)
      [retry_after] = get_resp_header(conn, "retry-after")
      assert String.to_integer(retry_after) in 1..60
    end
  end
end
