defmodule AuthWeb.Plugs.InternalToken do
  @moduledoc """
  Shared-secret gate for world → auth S2S (x0034).
  Header: `x-alchemy-internal-token`.
  """

  import Plug.Conn
  import Phoenix.Controller

  def init(opts), do: opts

  def call(conn, _opts) do
    expected = Application.get_env(:auth, :internal_token)

    presented =
      case get_req_header(conn, "x-alchemy-internal-token") do
        [token | _] -> token
        _ -> nil
      end

    cond do
      not is_binary(expected) or expected == "" ->
        conn
        |> put_status(:service_unavailable)
        |> json(%{error: "internal_token_unconfigured"})
        |> halt()

      not is_binary(presented) or presented == "" ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "missing_internal_token"})
        |> halt()

      not secure_compare(expected, presented) ->
        conn
        |> put_status(:unauthorized)
        |> json(%{error: "invalid_internal_token"})
        |> halt()

      true ->
        conn
    end
  end

  defp secure_compare(a, b) when byte_size(a) == byte_size(b) do
    Plug.Crypto.secure_compare(a, b)
  end

  defp secure_compare(_, _), do: false
end
