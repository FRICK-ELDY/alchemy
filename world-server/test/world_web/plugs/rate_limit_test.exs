defmodule WorldWeb.Plugs.RateLimitTest do
  use WorldWeb.ConnCase, async: false

  alias World.RateLimit
  alias World.TestAuthHelpers

  @strict_limits %{
    room_create: %{
      ip: %{limit: 2, period_ms: 60_000},
      user: %{limit: 2, period_ms: 60_000}
    },
    room_token: %{
      ip: %{limit: 2, period_ms: 60_000},
      user: %{limit: 2, period_ms: 60_000}
    },
    room_visibility: %{
      ip: %{limit: 2, period_ms: 60_000},
      user: %{limit: 2, period_ms: 60_000}
    }
  }

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    RateLimit.reset()
    previous = Application.get_env(:world, RateLimit)

    Application.put_env(:world, RateLimit, Keyword.put(previous || [], :limits, @strict_limits))

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, RateLimit)
        value -> Application.put_env(:world, RateLimit, value)
      end

      RateLimit.reset()
    end)

    %{signer: signer}
  end

  test "returns 429 when room_create user limit is exceeded", %{conn: conn, signer: signer} do
    sub = "44444444-4444-4444-4444-444444444444"

    for i <- 1..2 do
      c =
        build_conn()
        |> TestAuthHelpers.put_auth(signer, sub)
        |> post("/api/v1/rooms", %{room_id: "rl-create-#{i}"})

      assert json_response(c, 201)
    end

    conn =
      conn
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms", %{room_id: "rl-create-overflow"})

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(conn, 429)
    assert Plug.Conn.get_resp_header(conn, "retry-after") != []
  end

  test "returns 429 when room_token user limit is exceeded", %{conn: conn, signer: signer} do
    sub = "55555555-5555-5555-5555-555555555555"

    assert %{"room_id" => "rl-token-room"} =
             conn
             |> TestAuthHelpers.put_auth(signer, sub)
             |> post("/api/v1/rooms", %{room_id: "rl-token-room"})
             |> json_response(201)

    RateLimit.reset()

    session_pub = Base.url_encode64(:crypto.strong_rand_bytes(32), padding: false)

    for _ <- 1..2 do
      c =
        build_conn()
        |> TestAuthHelpers.put_auth(signer, sub)
        |> post("/api/v1/rooms/rl-token-room/tokens", %{
          role: "guest",
          session_pub: session_pub
        })

      assert json_response(c, 200)
    end

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post("/api/v1/rooms/rl-token-room/tokens", %{
        role: "guest",
        session_pub: session_pub
      })

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(conn, 429)
  end
end
