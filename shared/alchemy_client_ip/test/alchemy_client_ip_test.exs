defmodule AlchemyClientIpTest do
  use ExUnit.Case, async: true

  import Plug.Conn
  import Plug.Test

  @otp_app :alchemy_client_ip

  setup do
    previous = Application.get_env(@otp_app, :trusted_proxies, [])

    on_exit(fn ->
      Application.put_env(@otp_app, :trusted_proxies, previous)
    end)

    :ok
  end

  test "without trusted proxies uses peer IP" do
    Application.put_env(@otp_app, :trusted_proxies, [])
    conn = conn(:get, "/") |> Map.put(:remote_ip, {203, 0, 113, 10})
    assert AlchemyClientIp.from_conn(conn, otp_app: @otp_app) == "203.0.113.10"
  end

  test "trusted proxy prefers CF-Connecting-IP" do
    Application.put_env(@otp_app, :trusted_proxies, ["127.0.0.1"])

    conn =
      conn(:get, "/")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> put_req_header("cf-connecting-ip", "198.51.100.7")
      |> put_req_header("x-forwarded-for", "203.0.113.99")

    assert AlchemyClientIp.from_conn(conn, otp_app: @otp_app) == "198.51.100.7"
  end

  test "trusted proxy falls back to X-Forwarded-For" do
    Application.put_env(@otp_app, :trusted_proxies, ["127.0.0.1"])

    conn =
      conn(:get, "/")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> put_req_header("x-forwarded-for", "203.0.113.50")

    assert AlchemyClientIp.from_conn(conn, otp_app: @otp_app) == "203.0.113.50"
  end

  test "untrusted peer ignores CF-Connecting-IP" do
    Application.put_env(@otp_app, :trusted_proxies, ["127.0.0.1"])

    conn =
      conn(:get, "/")
      |> Map.put(:remote_ip, {203, 0, 113, 10})
      |> put_req_header("cf-connecting-ip", "198.51.100.7")
      |> put_req_header("x-forwarded-for", "198.51.100.7")

    assert AlchemyClientIp.from_conn(conn, otp_app: @otp_app) == "203.0.113.10"
  end

  test "trusted proxy with invalid CF-Connecting-IP falls back to X-Forwarded-For" do
    Application.put_env(@otp_app, :trusted_proxies, ["127.0.0.1"])

    conn =
      conn(:get, "/")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> put_req_header("cf-connecting-ip", "not-an-ip")
      |> put_req_header("x-forwarded-for", "203.0.113.50")

    assert AlchemyClientIp.from_conn(conn, otp_app: @otp_app) == "203.0.113.50"
  end
end
