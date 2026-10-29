defmodule AuthWeb.ClientIpTest do
  @moduledoc """
  Thin smoke that auth wires `otp_app: :auth`. Full matrix lives in
  `shared/alchemy_client_ip`.
  """

  use ExUnit.Case, async: true

  import Plug.Test

  setup do
    previous = Application.get_env(:auth, :trusted_proxies, [])

    on_exit(fn ->
      Application.put_env(:auth, :trusted_proxies, previous)
    end)

    :ok
  end

  test "delegates with otp_app :auth (trusted proxy + CF)" do
    Application.put_env(:auth, :trusted_proxies, ["127.0.0.1"])

    conn =
      conn(:get, "/")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> Plug.Conn.put_req_header("cf-connecting-ip", "198.51.100.7")

    assert AuthWeb.ClientIp.from_conn(conn) == "198.51.100.7"
  end
end
