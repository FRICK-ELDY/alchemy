defmodule WorldWeb.Plugs.RateLimitTest do
  use WorldWeb.ConnCase, async: false

  alias World.RateLimit
  alias World.TestAuthHelpers

  @high %{limit: 10_000, period_ms: 60_000}
  @strict_two %{limit: 2, period_ms: 60_000}

  @user_auth_strict_limits %{
    room_create: %{ip: @strict_two, user: @strict_two},
    room_token: %{ip: @strict_two, user: @strict_two},
    room_visibility: %{ip: @strict_two, user: @strict_two},
    host_seal: %{ip: @high, jti: @high},
    host_scene: %{ip: @high, jti: @high},
    place_read: %{ip: @high, jti: @high}
  }

  @host_token_strict_limits %{
    room_create: %{ip: @high, user: @high},
    room_token: %{ip: @high, user: @high},
    room_visibility: %{ip: @high, user: @high},
    host_seal: %{ip: @high, jti: @strict_two},
    host_scene: %{ip: @high, jti: @strict_two},
    place_read: %{ip: @high, jti: @strict_two}
  }

  @host_ip_strict_limits %{
    room_create: %{ip: @high, user: @high},
    room_token: %{ip: @high, user: @high},
    room_visibility: %{ip: @high, user: @high},
    host_seal: %{ip: @strict_two, jti: @high},
    host_scene: %{ip: @high, jti: @high},
    place_read: %{ip: @high, jti: @high}
  }

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    RateLimit.reset()
    previous = Application.get_env(:world, RateLimit)

    Application.put_env(
      :world,
      RateLimit,
      Keyword.put(previous || [], :limits, @user_auth_strict_limits)
    )

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:world, RateLimit)
        value -> Application.put_env(:world, RateLimit, value)
      end

      RateLimit.reset()
    end)

    %{signer: signer, previous: previous}
  end

  defp put_limits(limits) do
    previous = Application.get_env(:world, RateLimit)
    Application.put_env(:world, RateLimit, Keyword.put(previous || [], :limits, limits))
    RateLimit.reset()
  end

  defp token_params(extra) when is_map(extra) do
    Map.merge(
      %{
        role: "guest",
        session_pub: Base.url_encode64(:crypto.strong_rand_bytes(32), padding: false)
      },
      extra
    )
  end

  defp create_room_and_host_token(signer, sub, room_id) do
    {pub, priv} = TestAuthHelpers.session_keypair()

    assert %{"room_id" => ^room_id} =
             build_conn()
             |> TestAuthHelpers.put_auth(signer, sub)
             |> post("/api/v1/rooms", %{room_id: room_id})
             |> json_response(201)

    %{"token" => host_token} =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> post(
        "/api/v1/rooms/#{room_id}/tokens",
        token_params(%{role: "host", session_pub: TestAuthHelpers.session_pub_b64(pub)})
      )
      |> json_response(200)

    {host_token, priv}
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

  test "returns 429 when room_visibility user limit is exceeded", %{conn: conn, signer: signer} do
    sub = "77777777-7777-7777-7777-777777777777"

    assert %{"room_id" => "rl-vis-room"} =
             conn
             |> TestAuthHelpers.put_auth(signer, sub)
             |> post("/api/v1/rooms", %{room_id: "rl-vis-room"})
             |> json_response(201)

    RateLimit.reset()

    for _ <- 1..2 do
      c =
        build_conn()
        |> TestAuthHelpers.put_auth(signer, sub)
        |> put("/api/v1/rooms/rl-vis-room/visibility", %{visibility: "public"})

      assert json_response(c, 200)
    end

    conn =
      build_conn()
      |> TestAuthHelpers.put_auth(signer, sub)
      |> put("/api/v1/rooms/rl-vis-room/visibility", %{visibility: "friends"})

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(conn, 429)
  end

  test "returns 429 when host_seal jti limit is exceeded (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848401"
    room = "rl-host-seal"
    {host_token, priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)
    path = "/api/v1/rooms/#{room}/host_keys"
    body = %{host_pub: host_pub}

    for _ <- 1..2 do
      c = TestAuthHelpers.post_host_json(build_conn(), host_token, priv, path, body)
      assert json_response(c, 200)
    end

    overflow =
      TestAuthHelpers.post_host_json(build_conn(), host_token, priv, path, body)

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
    assert Plug.Conn.get_resp_header(overflow, "retry-after") != []
  end

  test "host_keys and manifest_seal share host_seal jti budget (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848405"
    room = "rl-host-seal-share"
    {host_token, priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)

    assert %{"attestation" => _} =
             build_conn()
             |> then(
               &TestAuthHelpers.post_host_json(
                 &1,
                 host_token,
                 priv,
                 "/api/v1/rooms/#{room}/host_keys",
                 %{host_pub: host_pub}
               )
             )
             |> json_response(200)

    # Second hit on the sibling route still counts against host_seal (body may fail authz).
    seal_hit =
      TestAuthHelpers.post_host_json(
        build_conn(),
        host_token,
        priv,
        "/api/v1/rooms/#{room}/manifest_seal",
        %{
          revision: 0,
          host_generation: 1,
          manifest_blake3: String.duplicate("a", 64),
          host_attestation: "not-an-attestation"
        }
      )

    refute seal_hit.status == 429
    assert seal_hit.status in [401, 403, 422]

    overflow =
      TestAuthHelpers.post_host_json(
        build_conn(),
        host_token,
        priv,
        "/api/v1/rooms/#{room}/host_keys",
        %{host_pub: host_pub}
      )

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
  end

  test "host_seal jti key follows claims room_id, not path (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848406"
    room = "rl-host-path"
    {host_token, priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)

    for path_room <- [room, "other-room-path"] do
      c =
        TestAuthHelpers.post_host_json(
          build_conn(),
          host_token,
          priv,
          "/api/v1/rooms/#{path_room}/host_keys",
          %{host_pub: host_pub}
        )

      # First path succeeds; rotated path is rejected by controller but still hits same jti key.
      assert c.status in [200, 403]
      refute c.status == 429
    end

    overflow =
      TestAuthHelpers.post_host_json(
        build_conn(),
        host_token,
        priv,
        "/api/v1/rooms/yet-another/host_keys",
        %{host_pub: host_pub}
      )

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
  end

  test "host_seal and host_scene use separate buckets (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848402"
    room = "rl-host-sep"
    {host_token, priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    host_pub = :crypto.strong_rand_bytes(32) |> Base.url_encode64(padding: false)
    seal_path = "/api/v1/rooms/#{room}/host_keys"

    for _ <- 1..2 do
      assert json_response(
               TestAuthHelpers.post_host_json(
                 build_conn(),
                 host_token,
                 priv,
                 seal_path,
                 %{host_pub: host_pub}
               ),
               200
             )
    end

    assert %{"errors" => %{"detail" => "Too Many Requests"}} =
             build_conn()
             |> then(
               &TestAuthHelpers.post_host_json(
                 &1,
                 host_token,
                 priv,
                 seal_path,
                 %{host_pub: host_pub}
               )
             )
             |> json_response(429)

    scene =
      TestAuthHelpers.get_host(
        build_conn(),
        host_token,
        priv,
        "/api/v1/rooms/#{room}/scene"
      )

    assert json_response(scene, 200)
  end

  test "returns 429 when place_read jti limit is exceeded (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848403"
    room = "rl-place-read"
    {host_token, _priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    for _ <- 1..2 do
      c =
        build_conn()
        |> put_req_header("authorization", "Bearer #{host_token}")
        |> post("/api/v1/rooms/#{room}/place_read", %{world_seal: "not-a-seal"})

      assert c.status in [401, 403, 422]
      refute c.status == 429
    end

    overflow =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> post("/api/v1/rooms/#{room}/place_read", %{world_seal: "not-a-seal"})

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
    assert Plug.Conn.get_resp_header(overflow, "retry-after") != []
  end

  test "returns 429 when place_hashes jti limit is exceeded (x0084)", %{signer: signer} do
    put_limits(@host_token_strict_limits)
    sub = "84848484-8484-8484-8484-848484848407"
    room = "rl-place-hashes"
    {host_token, _priv} = create_room_and_host_token(signer, sub, room)
    RateLimit.reset()

    # Cover GET place_hashes route (Bearer may be wrong typ → 401; still counts jti).
    for _ <- 1..2 do
      c =
        build_conn()
        |> put_req_header("authorization", "Bearer #{host_token}")
        |> get("/api/v1/rooms/#{room}/place_hashes")

      assert c.status in [401, 403]
      refute c.status == 429
    end

    overflow =
      build_conn()
      |> put_req_header("authorization", "Bearer #{host_token}")
      |> get("/api/v1/rooms/#{room}/place_hashes")

    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
    assert Plug.Conn.get_resp_header(overflow, "retry-after") != []
  end

  test "returns 429 on host_seal IP axis without Bearer (x0084)", %{signer: _signer} do
    put_limits(@host_ip_strict_limits)
    RateLimit.reset()

    for _ <- 1..2 do
      c = build_conn() |> post("/api/v1/rooms/rl-ip-seal/host_keys", %{host_pub: "x"})
      assert c.status in [401, 422]
      refute c.status == 429
    end

    overflow = build_conn() |> post("/api/v1/rooms/rl-ip-seal/host_keys", %{host_pub: "x"})
    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(overflow, 429)
    assert Plug.Conn.get_resp_header(overflow, "retry-after") != []
  end
end
