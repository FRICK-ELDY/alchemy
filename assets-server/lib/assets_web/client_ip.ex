defmodule AssetsWeb.ClientIp do
  @moduledoc """
  Trusted-proxy client IP for assets HTTP rate limits (x00d9 / x004b).
  """

  @spec from_conn(Plug.Conn.t()) :: String.t()
  def from_conn(conn) do
    AlchemyClientIp.from_conn(conn, otp_app: :assets)
  end
end
