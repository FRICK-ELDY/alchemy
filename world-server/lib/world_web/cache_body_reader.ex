defmodule WorldWeb.CacheBodyReader do
  @moduledoc """
  Stash raw request body for HTTP PoP (x0081) before JSON parsing.
  """

  @doc false
  def read_body(conn, opts) do
    {:ok, body, conn} = Plug.Conn.read_body(conn, opts)
    conn = Plug.Conn.assign(conn, :raw_body, body)
    {:ok, body, conn}
  end
end
