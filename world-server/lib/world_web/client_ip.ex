defmodule WorldWeb.ClientIp do
  @moduledoc """
  Client IP resolution (mirrors auth x002e). Used for `lan` visibility (x0036).
  """

  @spec from_conn(Plug.Conn.t()) :: String.t()
  def from_conn(conn) do
    peer = conn.remote_ip
    proxies = Application.get_env(:world, :trusted_proxies, [])

    cond do
      proxies == [] ->
        format(peer)

      trusted_proxy?(peer, proxies) ->
        client_ip_behind_proxy(conn, peer, proxies)

      true ->
        format(peer)
    end
  end

  @spec via_cloudflare_tunnel?(Plug.Conn.t()) :: boolean()
  def via_cloudflare_tunnel?(conn) do
    match?([_ | _], Plug.Conn.get_req_header(conn, "cf-connecting-ip")) or
      match?([_ | _], Plug.Conn.get_req_header(conn, "cf-ray"))
  end

  @doc """
  True when the resolved client IP is loopback / link-local / RFC1918 / ULA,
  and the request is not marked as Cloudflare Tunnel ingress.
  """
  @spec lan_eligible?(Plug.Conn.t()) :: boolean()
  def lan_eligible?(conn) do
    if via_cloudflare_tunnel?(conn) do
      false
    else
      private_or_local?(from_conn(conn))
    end
  end

  @spec private_or_local?(String.t()) :: boolean()
  def private_or_local?(ip) when is_binary(ip) do
    case :inet.parse_address(String.to_charlist(ip)) do
      {:ok, addr} -> private_or_local_addr?(addr)
      {:error, _} -> false
    end
  end

  def private_or_local?(_), do: false

  defp private_or_local_addr?({127, _, _, _}), do: true
  defp private_or_local_addr?({10, _, _, _}), do: true
  defp private_or_local_addr?({192, 168, _, _}), do: true
  defp private_or_local_addr?({172, b, _, _}) when b >= 16 and b <= 31, do: true
  defp private_or_local_addr?({169, 254, _, _}), do: true
  # IPv6 loopback
  defp private_or_local_addr?({0, 0, 0, 0, 0, 0, 0, 1}), do: true
  # ULA fc00::/7 → first hextet 0xFC00..0xFDFF
  defp private_or_local_addr?({h, _, _, _, _, _, _, _})
       when is_integer(h) and h >= 0xFC00 and h <= 0xFDFF,
       do: true

  # link-local fe80::/10 → first hextet 0xFE80..0xFEBF
  defp private_or_local_addr?({h, _, _, _, _, _, _, _})
       when is_integer(h) and h >= 0xFE80 and h <= 0xFEBF,
       do: true

  defp private_or_local_addr?(_), do: false

  defp client_ip_behind_proxy(conn, peer, proxies) do
    case cf_connecting_ip(conn) do
      {:ok, ip} ->
        format(ip)

      :error ->
        conn.req_headers
        |> RemoteIp.from(headers: ~w[x-forwarded-for], proxies: proxies)
        |> case do
          nil -> format(peer)
          ip -> format(ip)
        end
    end
  end

  defp cf_connecting_ip(conn) do
    case Plug.Conn.get_req_header(conn, "cf-connecting-ip") do
      [raw | _] ->
        raw = String.trim(raw)

        case :inet.parse_address(String.to_charlist(raw)) do
          {:ok, ip} -> {:ok, ip}
          {:error, _} -> :error
        end

      _ ->
        :error
    end
  end

  defp trusted_proxy?(peer, proxies) do
    Enum.any?(proxies, &proxy_match?(&1, peer))
  end

  defp proxy_match?(proxy, peer) do
    case RemoteIp.Block.parse(proxy) do
      {:ok, block} -> RemoteIp.Block.contains?(block, RemoteIp.Block.encode(peer))
      _ -> format(peer) == proxy
    end
  end

  defp format(ip) do
    ip
    |> :inet.ntoa()
    |> to_string()
  end
end
