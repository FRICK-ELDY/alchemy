defmodule WorldWeb.CacheBodyReader do
  @moduledoc """
  Stash raw request body for HTTP PoP (x0081) before JSON parsing.
  """

  @doc false
  def read_body(conn, opts) do
    case Plug.Conn.read_body(conn, opts) do
      {:ok, body, conn} ->
        # PoP signs the complete body only — do not stash partials from {:more, …}.
        {:ok, body, Plug.Conn.assign(conn, :raw_body, body)}

      other ->
        other
    end
  end
end
