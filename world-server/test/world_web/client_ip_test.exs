defmodule WorldWeb.ClientIpTest do
  use ExUnit.Case, async: true

  alias WorldWeb.ClientIp

  test "private_or_local? covers RFC1918 and loopback" do
    assert ClientIp.private_or_local?("127.0.0.1")
    assert ClientIp.private_or_local?("10.0.0.1")
    assert ClientIp.private_or_local?("192.168.0.1")
    assert ClientIp.private_or_local?("172.16.5.1")
    refute ClientIp.private_or_local?("8.8.8.8")
    refute ClientIp.private_or_local?("172.32.0.1")
  end

  test "private_or_local? covers IPv6 ULA /7 and link-local /10" do
    assert ClientIp.private_or_local?("::1")
    # ULA examples across fc00::/7
    assert ClientIp.private_or_local?("fc00::1")
    assert ClientIp.private_or_local?("fd12:3456:789a::1")
    assert ClientIp.private_or_local?("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")
    # link-local fe80::/10
    assert ClientIp.private_or_local?("fe80::1")
    assert ClientIp.private_or_local?("febf::1")
    refute ClientIp.private_or_local?("fec0::1")
    refute ClientIp.private_or_local?("2001:db8::1")
  end

  test "lan_eligible? false behind CF headers" do
    conn =
      Plug.Test.conn(:get, "/")
      |> Map.put(:remote_ip, {10, 0, 0, 1})
      |> Plug.Conn.put_req_header("cf-ray", "abc")

    refute ClientIp.lan_eligible?(conn)
  end
end
