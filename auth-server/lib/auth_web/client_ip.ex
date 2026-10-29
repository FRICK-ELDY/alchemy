defmodule AuthWeb.ClientIp do
  @moduledoc false

  @spec from_conn(Plug.Conn.t()) :: String.t()
  def from_conn(conn) do
    AlchemyClientIp.from_conn(conn, otp_app: :auth)
  end
end
