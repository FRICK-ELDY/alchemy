defmodule AlchemyAuthClient.JwksTest do
  use ExUnit.Case, async: false

  alias AlchemyAuthClient.Jwks

  @otp_app :alchemy_auth_client
  @server __MODULE__.JwksServer

  setup do
    bypass = Bypass.open()
    url = "http://127.0.0.1:#{bypass.port}/.well-known/jwks.json"

    previous_url = Application.get_env(@otp_app, :auth_jwks_url)
    previous_fetch = Application.get_env(@otp_app, :auth_jwks_fetch_on_start)

    Application.put_env(@otp_app, :auth_jwks_url, url)
    Application.put_env(@otp_app, :auth_jwks_fetch_on_start, false)

    {:ok, clock} = Agent.start_link(fn -> 0 end)

    on_exit(fn ->
      if previous_url,
        do: Application.put_env(@otp_app, :auth_jwks_url, previous_url),
        else: Application.delete_env(@otp_app, :auth_jwks_url)

      if previous_fetch,
        do: Application.put_env(@otp_app, :auth_jwks_fetch_on_start, previous_fetch),
        else: Application.delete_env(@otp_app, :auth_jwks_fetch_on_start)

      if Process.whereis(@server), do: GenServer.stop(@server)
      if Process.alive?(clock), do: Agent.stop(clock)
    end)

    {:ok, _pid} =
      Jwks.start_link(
        otp_app: @otp_app,
        name: @server,
        fetch_cooldown_ms: 1_000,
        now_ms: fn -> Agent.get(clock, & &1) end
      )

    %{bypass: bypass, clock: clock}
  end

  test "install_test_jwks! serves signer_for_kid", %{bypass: bypass, clock: clock} do
    Bypass.stub(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      Plug.Conn.resp(conn, 500, "unused")
    end)

    {key, signer, kid} = rsa_jwks_entry()

    Agent.update(clock, fn _ -> 1_000 end)
    assert :ok = Jwks.install_test_jwks!(@server, %{"keys" => [key]}, %{kid => signer})
    assert {:ok, ^signer} = Jwks.signer_for_kid(@server, kid)

    # Within cooldown after install — miss must not kick HTTP (flake vs on_exit).
    assert {:error, :unknown_kid} = Jwks.signer_for_kid(@server, "missing")
    assert :ok = Jwks.await_idle(@server)
  end

  test "unknown kid within cooldown does not re-fetch", %{bypass: bypass, clock: clock} do
    hits = :counters.new(1, [])
    {key, _signer, kid} = rsa_jwks_entry()
    body = Jason.encode!(%{"keys" => [key]})

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)

      conn
      |> Plug.Conn.put_resp_content_type("application/json")
      |> Plug.Conn.resp(200, body)
    end)

    Agent.update(clock, fn _ -> 1_000 end)
    assert :ok = Jwks.refresh(@server)
    assert :counters.get(hits, 1) == 1
    assert {:ok, _} = Jwks.signer_for_kid(@server, kid)

    # Still inside 1000ms cooldown window.
    Agent.update(clock, fn _ -> 1_999 end)
    assert {:error, :unknown_kid} = Jwks.signer_for_kid(@server, "missing-a")
    assert {:error, :unknown_kid} = Jwks.signer_for_kid(@server, "missing-b")
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 1
  end

  test "unknown kid re-fetches after cooldown elapses", %{bypass: bypass, clock: clock} do
    hits = :counters.new(1, [])
    {key, _signer, kid} = rsa_jwks_entry()
    body = Jason.encode!(%{"keys" => [key]})

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)

      conn
      |> Plug.Conn.put_resp_content_type("application/json")
      |> Plug.Conn.resp(200, body)
    end)

    Agent.update(clock, fn _ -> 1_000 end)
    assert :ok = Jwks.refresh(@server)
    assert :counters.get(hits, 1) == 1
    assert {:ok, _} = Jwks.signer_for_kid(@server, kid)

    Agent.update(clock, fn _ -> 1_500 end)
    assert {:error, :unknown_kid} = Jwks.signer_for_kid(@server, "missing-early")
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 1

    Agent.update(clock, fn _ -> 2_000 end)
    assert {:error, :unknown_kid} = Jwks.signer_for_kid(@server, "missing-late")
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 2
  end

  test "async miss success is visible on the next signer_for_kid", %{
    bypass: bypass,
    clock: clock
  } do
    hits = :counters.new(1, [])
    {key, _signer, kid} = rsa_jwks_entry()
    body = Jason.encode!(%{"keys" => [key]})

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)

      conn
      |> Plug.Conn.put_resp_content_type("application/json")
      |> Plug.Conn.resp(200, body)
    end)

    Agent.update(clock, fn _ -> 1_000 end)
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, kid)
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 1

    assert {:ok, _} = Jwks.signer_for_kid(@server, kid)
    assert :counters.get(hits, 1) == 1
  end

  test "fetch failure stamps cooldown so miss storms do not re-HTTP", %{
    bypass: bypass,
    clock: clock
  } do
    hits = :counters.new(1, [])

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)
      Plug.Conn.resp(conn, 503, "down")
    end)

    Agent.update(clock, fn _ -> 1_000 end)
    assert {:error, _} = Jwks.refresh(@server)
    assert :counters.get(hits, 1) == 1

    Agent.update(clock, fn _ -> 1_500 end)
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "any-kid")
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "other-kid")
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 1

    Agent.update(clock, fn _ -> 2_000 end)
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "retry-kid")
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 2
  end

  test "signer_for_kid miss returns before slow HTTP completes", %{bypass: bypass, clock: clock} do
    hits = :counters.new(1, [])
    test_pid = self()

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)
      send(test_pid, {:fetch_started, self()})

      receive do
        :release -> :ok
      after
        5_000 -> flunk("fetch not released")
      end

      Plug.Conn.resp(conn, 503, "slow")
    end)

    Agent.update(clock, fn _ -> 1_000 end)

    task =
      Task.async(fn ->
        Jwks.signer_for_kid(@server, "missing")
      end)

    assert_receive {:fetch_started, handler}, 1_000
    # Call must have returned without waiting on the blocked Bypass handler.
    assert {:error, :empty_jwks} = Task.await(task, 200)
    assert :counters.get(hits, 1) == 1

    send(handler, :release)
    assert :ok = Jwks.await_idle(@server)
  end

  test "in-flight miss refresh coalesces", %{bypass: bypass, clock: clock} do
    hits = :counters.new(1, [])
    test_pid = self()

    Bypass.expect(bypass, "GET", "/.well-known/jwks.json", fn conn ->
      :counters.add(hits, 1, 1)
      send(test_pid, {:fetch_started, self()})

      receive do
        :go -> :ok
      after
        5_000 -> flunk("fetch not released")
      end

      Plug.Conn.resp(conn, 503, "down")
    end)

    Agent.update(clock, fn _ -> 1_000 end)
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "a")
    assert_receive {:fetch_started, handler}, 1_000

    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "b")
    assert {:error, :empty_jwks} = Jwks.signer_for_kid(@server, "c")

    send(handler, :go)
    assert :ok = Jwks.await_idle(@server)
    assert :counters.get(hits, 1) == 1
  end

  defp rsa_jwks_entry do
    jwk = JOSE.JWK.generate_key({:rsa, 2048})
    public = JOSE.JWK.to_public(jwk)
    {_fields, map} = JOSE.JWK.to_map(public)
    kid = "test-kid-" <> Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)

    key =
      map
      |> Map.put("kid", kid)
      |> Map.put("use", "sig")
      |> Map.put("alg", "RS256")

    signer =
      Joken.Signer.create("RS256", %{
        "kty" => "RSA",
        "n" => key["n"],
        "e" => key["e"],
        "kid" => kid
      })

    {key, signer, kid}
  end
end
