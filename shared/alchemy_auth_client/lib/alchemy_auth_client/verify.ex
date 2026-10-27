defmodule AlchemyAuthClient.Verify do
  @moduledoc """
  Verifies auth-server user access JWTs (RS256) via a JWKS GenServer.
  """

  @type server :: GenServer.server()

  @doc """
  Verifies `token` using JWKS at `jwks_server` and issuer/audience from `otp_app`.

  Reads `:auth_jwt_issuer` and `:auth_jwt_audience` from `otp_app`.
  """
  @spec verify(String.t(), keyword()) :: {:ok, map()} | {:error, term()}
  def verify(token, opts) when is_binary(token) and token != "" and is_list(opts) do
    otp_app = Keyword.fetch!(opts, :otp_app)
    jwks_server = Keyword.get(opts, :jwks, AlchemyAuthClient.Jwks)

    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- AlchemyAuthClient.Jwks.signer_for_kid(jwks_server, kid),
         {:ok, claims} <- Joken.verify_and_validate(token_config(otp_app), token, signer) do
      ensure_active(claims)
    else
      {:error, reason} -> {:error, reason}
      other -> {:error, {:unexpected, other}}
    end
  end

  def verify(_, _), do: {:error, :missing_token}

  defp token_config(otp_app) do
    issuer = Application.fetch_env!(otp_app, :auth_jwt_issuer)
    audience = Application.fetch_env!(otp_app, :auth_jwt_audience)

    Joken.Config.default_claims(skip: [:aud, :iss])
    |> Joken.Config.add_claim("iss", fn -> issuer end, &(&1 == issuer))
    |> Joken.Config.add_claim("aud", fn -> audience end, &aud_matches?(&1, audience))
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
  end

  defp aud_matches?(aud, expected) when is_binary(aud), do: aud == expected
  defp aud_matches?(aud, expected) when is_list(aud), do: expected in aud
  defp aud_matches?(_, _), do: false

  defp peek_kid(token) do
    case Joken.peek_header(token) do
      {:ok, %{"kid" => kid}} when is_binary(kid) and kid != "" ->
        {:ok, %{"kid" => kid}}

      {:ok, _} ->
        {:error, :missing_kid}

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp ensure_active(%{"status" => "active"} = claims), do: {:ok, claims}
  defp ensure_active(%{"status" => status}) when is_binary(status), do: {:error, :inactive_user}
  defp ensure_active(claims) when is_map(claims), do: {:ok, claims}
end
