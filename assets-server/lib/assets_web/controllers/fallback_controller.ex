defmodule AssetsWeb.FallbackController do
  use AssetsWeb, :controller

  def call(conn, {:error, :not_found}) do
    conn
    |> put_status(:not_found)
    |> json(%{error: "not_found"})
  end

  def call(conn, _error) do
    conn
    |> put_status(:internal_server_error)
    |> json(%{error: "internal_error"})
  end
end
