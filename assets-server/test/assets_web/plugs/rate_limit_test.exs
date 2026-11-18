defmodule AssetsWeb.Plugs.RateLimitTest do
  use AssetsWeb.ConnCase, async: false, cas: true

  alias Assets.RateLimit
  alias Assets.TestAuthHelpers
  alias Assets.Vcs.PushGate

  @high %{limit: 10_000, period_ms: 60_000}
  @strict_two %{limit: 2, period_ms: 60_000}

  @strict_all %{
    blob_put: %{ip: @strict_two, user: @strict_two},
    vcs_negotiate: %{ip: @strict_two, user: @strict_two},
    vcs_push: %{ip: @strict_two, user: @strict_two},
    spawn_ticket: %{ip: @strict_two, user: @strict_two}
  }

  @blob_only_strict %{
    blob_put: %{ip: @strict_two, user: @strict_two},
    vcs_negotiate: %{ip: @high, user: @high},
    vcs_push: %{ip: @high, user: @high},
    spawn_ticket: %{ip: @high, user: @high}
  }

  @spawn_ip_strict %{
    blob_put: %{ip: @high, user: @high},
    vcs_negotiate: %{ip: @high, user: @high},
    vcs_push: %{ip: @high, user: @high},
    spawn_ticket: %{ip: @strict_two, user: @high}
  }

  @hash64 String.duplicate("ab", 32)
  @repo_id "00000000-0000-0000-0000-000000000001"

  setup do
    %{signer: signer} = TestAuthHelpers.install_test_auth!()
    RateLimit.reset()
    previous = Application.get_env(:assets, RateLimit)

    Application.put_env(
      :assets,
      RateLimit,
      Keyword.put(previous || [], :limits, @strict_all)
    )

    on_exit(fn ->
      case previous do
        nil -> Application.delete_env(:assets, RateLimit)
        value -> Application.put_env(:assets, RateLimit, value)
      end

      RateLimit.reset()
    end)

    %{signer: signer, previous: previous}
  end

  defp put_limits(limits) do
    previous = Application.get_env(:assets, RateLimit)
    Application.put_env(:assets, RateLimit, Keyword.put(previous || [], :limits, limits))
    RateLimit.reset()
  end

  defp auth_json(conn, signer, sub) do
    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> put_req_header("content-type", "application/json")
  end

  defp post_spawn(conn, signer, sub) do
    conn
    |> auth_json(signer, sub)
    |> post("/api/v1/spawn_tickets", Jason.encode!(%{room_id: "local", blake3s: [@hash64]}))
  end

  defp post_blob(conn, signer, sub, path \\ "/api/v1/blobs") do
    conn
    |> TestAuthHelpers.put_auth(signer, sub)
    |> put_req_header("content-type", "application/octet-stream")
    |> post(path, "rl-bytes")
  end

  defp post_negotiate(conn, signer, sub) do
    conn
    |> auth_json(signer, sub)
    |> post(
      "/api/v1/vcs/repos/#{@repo_id}/negotiate",
      Jason.encode!(%{
        ref_name: "main",
        new_commit_hash: @hash64,
        object_hashes: [],
        blob_blake3s: []
      })
    )
  end

  defp post_push(conn, signer, sub) do
    conn
    |> auth_json(signer, sub)
    |> post(
      "/api/v1/vcs/repos/#{@repo_id}/push",
      Jason.encode!(%{
        ref_name: "main",
        new_commit_hash: @hash64,
        objects: []
      })
    )
  end

  defp assert_rate_limited(conn) do
    assert %{"errors" => %{"detail" => "Too Many Requests"}} = json_response(conn, 429)
    assert Plug.Conn.get_resp_header(conn, "retry-after") != []
  end

  test "returns 429 when spawn_ticket user limit is exceeded", %{signer: signer} do
    sub = "11111111-1111-1111-1111-111111111111"

    for _ <- 1..2 do
      # May be 403 no_grant — RL still counts after auth.
      _ = post_spawn(build_conn(), signer, sub)
    end

    assert_rate_limited(post_spawn(build_conn(), signer, sub))
  end

  test "returns 429 when blob_put user limit is exceeded", %{signer: signer} do
    sub = "22222222-2222-2222-2222-222222222222"
    # cas: true — uploads may touch CAS; we only assert the 3rd call is 429.
    put_limits(@blob_only_strict)

    for _ <- 1..2 do
      c = post_blob(build_conn(), signer, sub)
      # 200/413/etc — only care that RL did not halt yet
      refute c.status == 429
    end

    assert_rate_limited(post_blob(build_conn(), signer, sub))
  end

  test "blob and inventory share blob_put bucket", %{signer: signer} do
    sub = "33333333-3333-3333-3333-333333333333"
    put_limits(@blob_only_strict)

    refute post_blob(build_conn(), signer, sub, "/api/v1/blobs").status == 429
    refute post_blob(build_conn(), signer, sub, "/api/v1/inventory/blobs").status == 429
    assert_rate_limited(post_blob(build_conn(), signer, sub, "/api/v1/blobs"))
  end

  test "returns 429 when vcs_negotiate user limit is exceeded", %{signer: signer} do
    sub = "44444444-4444-4444-4444-444444444444"

    for _ <- 1..2 do
      _ = post_negotiate(build_conn(), signer, sub)
    end

    assert_rate_limited(post_negotiate(build_conn(), signer, sub))
  end

  test "returns 429 when vcs_push user limit is exceeded", %{signer: signer} do
    sub = "55555555-5555-5555-5555-555555555555"

    for _ <- 1..2 do
      _ = post_push(build_conn(), signer, sub)
    end

    assert_rate_limited(post_push(build_conn(), signer, sub))
  end

  test "blob_put exhaustion does not block spawn_ticket", %{signer: signer} do
    sub = "66666666-6666-6666-6666-666666666666"
    put_limits(@blob_only_strict)

    for _ <- 1..2 do
      _ = post_blob(build_conn(), signer, sub)
    end

    assert_rate_limited(post_blob(build_conn(), signer, sub))

    spawn = post_spawn(build_conn(), signer, sub)
    refute spawn.status == 429
  end

  test "returns 429 on IP axis across different subs", %{signer: signer} do
    put_limits(@spawn_ip_strict)
    sub_a = "77777777-7777-7777-7777-777777777777"
    sub_b = "88888888-8888-8888-8888-888888888888"

    refute post_spawn(build_conn(), signer, sub_a).status == 429
    refute post_spawn(build_conn(), signer, sub_b).status == 429
    assert_rate_limited(post_spawn(build_conn(), signer, sub_a))
  end

  test "push_in_flight when same sub holds PushGate; other sub can checkout", %{signer: signer} do
    put_limits(%{
      blob_put: %{ip: @high, user: @high},
      vcs_negotiate: %{ip: @high, user: @high},
      vcs_push: %{ip: @high, user: @high},
      spawn_ticket: %{ip: @high, user: @high}
    })

    sub_a = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    sub_b = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"

    parent = self()

    holder =
      spawn(fn ->
        assert :ok = PushGate.checkout(sub_a)
        send(parent, :held)

        receive do
          :release -> PushGate.checkin(sub_a)
        end
      end)

    assert_receive :held, 1_000

    busy =
      post_push(build_conn(), signer, sub_a)

    assert %{"error" => "push_in_flight"} = json_response(busy, 429)
    assert Plug.Conn.get_resp_header(busy, "retry-after") == ["1"]
    assert Plug.Conn.get_resp_header(busy, "connection") == ["close"]

    assert :ok = PushGate.checkout(sub_b)
    assert :ok = PushGate.checkin(sub_b)

    send(holder, :release)
    ref = Process.monitor(holder)
    assert_receive {:DOWN, ^ref, :process, ^holder, _}, 1_000
  end

  test "PushGate releases on holder crash", %{signer: _signer} do
    sub = "cccccccc-cccc-cccc-cccc-cccccccccccc"
    parent = self()

    holder =
      spawn(fn ->
        assert :ok = PushGate.checkout(sub)
        send(parent, :held)

        receive do
          :never -> :ok
        end
      end)

    assert_receive :held, 1_000
    assert {:error, :busy} = PushGate.checkout(sub)

    ref = Process.monitor(holder)
    Process.exit(holder, :kill)
    assert_receive {:DOWN, ^ref, :process, ^holder, _}, 1_000
    # Drain PushGate mailbox so its {:DOWN,...} is applied before checkout.
    _ = :sys.get_state(PushGate)

    assert :ok = PushGate.checkout(sub)
    assert :ok = PushGate.checkin(sub)
  end
end
