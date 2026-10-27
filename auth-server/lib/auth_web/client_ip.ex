defmodule AuthWeb.ClientIp do
  @moduledoc false

  @spec from_conn(Plug.Conn.t()) :: String.t()
  def from_conn(conn) do
    peer = conn.remote_ip
    proxies = Application.get_env(:auth, :trusted_proxies, [])

    cond do
      proxies == [] ->
        format(peer)

      trusted_proxy?(peer, proxies) ->
        client_ip_behind_proxy(conn, peer, proxies)

      true ->
        format(peer)
    end
  end

  # Prefer CF-Connecting-IP (Tunnel / Cloudflare edge), then X-Forwarded-For (x002e).
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
