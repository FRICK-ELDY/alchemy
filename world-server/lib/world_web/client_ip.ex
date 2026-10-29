defmodule WorldWeb.ClientIp do
  @moduledoc """
  Client IP + `lan` visibility helpers (x0036).

  Trusted-proxy resolution is shared (`AlchemyClientIp`, x004b).
  """

  @spec from_conn(Plug.Conn.t()) :: String.t()
  def from_conn(conn) do
    AlchemyClientIp.from_conn(conn, otp_app: :world)
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
  defp private_or_local_addr?({0, 0, 0, 0, 0, 0, 0, 1}), do: true

  defp private_or_local_addr?({h, _, _, _, _, _, _, _})
       when is_integer(h) and h >= 0xFC00 and h <= 0xFDFF,
       do: true

  defp private_or_local_addr?({h, _, _, _, _, _, _, _})
       when is_integer(h) and h >= 0xFE80 and h <= 0xFEBF,
       do: true

  defp private_or_local_addr?(_), do: false
end
