defmodule World.RoomAccessTest do
  use ExUnit.Case, async: false

  alias World.RoomAccess
  alias WorldWeb.ClientIp

  setup do
    previous = Application.get_env(:world, :auth_access_stub)

    on_exit(fn ->
      Application.put_env(:world, :auth_access_stub, previous)
    end)

    :ok
  end

  test "owner always allowed even for private" do
    conn = Plug.Test.conn(:get, "/")
    assert :ok = RoomAccess.authorize(conn, "owner", "owner", "private")
  end

  test "private forbids non-owner" do
    conn = Plug.Test.conn(:get, "/")

    assert {:error, :forbidden_visibility} =
             RoomAccess.authorize(conn, "owner", "guest", "private")
  end

  test "public allows non-owner" do
    conn = Plug.Test.conn(:get, "/")
    assert :ok = RoomAccess.authorize(conn, "owner", "guest", "public")
  end

  test "friends uses auth stub" do
    Application.put_env(:world, :auth_access_stub, fn
      "owner", "friend", "friends" -> true
      _, _, _ -> false
    end)

    conn = Plug.Test.conn(:get, "/")
    assert :ok = RoomAccess.authorize(conn, "owner", "friend", "friends")

    assert {:error, :forbidden_visibility} =
             RoomAccess.authorize(conn, "owner", "stranger", "friends")
  end

  test "friends_plus / followers / followers_plus use stub" do
    Application.put_env(:world, :auth_access_stub, fn
      "o", "v", "friends_plus" -> true
      "o", "v", "followers" -> true
      "o", "v", "followers_plus" -> true
      _, _, _ -> false
    end)

    conn = Plug.Test.conn(:get, "/")
    assert :ok = RoomAccess.authorize(conn, "o", "v", "friends_plus")
    assert :ok = RoomAccess.authorize(conn, "o", "v", "followers")
    assert :ok = RoomAccess.authorize(conn, "o", "v", "followers_plus")
  end

  test "lan requires private IP and rejects cf-connecting-ip" do
    lan_conn = %{Plug.Test.conn(:get, "/") | remote_ip: {192, 168, 1, 10}}
    assert :ok = RoomAccess.authorize(lan_conn, "owner", "guest", "lan")

    wan_conn = %{Plug.Test.conn(:get, "/") | remote_ip: {8, 8, 8, 8}}
    assert {:error, :lan_required} = RoomAccess.authorize(wan_conn, "owner", "guest", "lan")

    tunnel =
      Plug.Test.conn(:get, "/")
      |> Map.put(:remote_ip, {192, 168, 1, 10})
      |> Plug.Conn.put_req_header("cf-connecting-ip", "203.0.113.9")

    assert ClientIp.via_cloudflare_tunnel?(tunnel)
    assert {:error, :lan_required} = RoomAccess.authorize(tunnel, "owner", "guest", "lan")
  end
end
