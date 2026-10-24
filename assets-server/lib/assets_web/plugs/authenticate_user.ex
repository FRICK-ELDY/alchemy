defmodule AssetsWeb.Plugs.AuthenticateUser do
  @moduledoc """
  Requires a valid auth-server Bearer JWT.
  """

  import Plug.Conn
  import Phoenix.Controller

  alias Assets.Auth.Verify

  def init(opts), do: opts

  def call(conn, _opts) do
    case fetch_bearer(conn) do
      {:ok, token} ->
        case Verify.verify(token) do
          {:ok, claims} ->
            conn
            |> assign(:auth_sub, claims["sub"])
            |> assign(:auth_claims, claims)

          {:error, :empty_jwks} ->
            unauthorized(conn, "auth_jwks_unavailable")

          {:error, _reason} ->
            unauthorized(conn, "invalid_auth_token")
        end

      :error ->
        unauthorized(conn, "missing_auth_token")
    end
  end

  defp fetch_bearer(conn) do
    case get_req_header(conn, "authorization") do
      ["Bearer " <> token] when byte_size(token) > 0 -> {:ok, token}
      _ -> :error
    end
  end

  defp unauthorized(conn, error) do
    conn
    |> put_status(:unauthorized)
    |> json(%{error: error})
    |> halt()
  end
end
