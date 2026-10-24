defmodule Assets.Auth.Verify do
  @moduledoc """
  Verifies auth-server user access JWTs (RS256) via cached JWKS.
  """

  alias Assets.Auth.Jwks

  @spec verify(String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token) when is_binary(token) and token != "" do
    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- Jwks.signer_for_kid(kid),
         {:ok, claims} <- Joken.verify_and_validate(token_config(), token, signer) do
      ensure_active(claims)
    else
      {:error, reason} -> {:error, reason}
      other -> {:error, {:unexpected, other}}
    end
  end

  def verify(_), do: {:error, :missing_token}

  defp token_config do
    issuer = Application.fetch_env!(:assets, :auth_jwt_issuer)
    audience = Application.fetch_env!(:assets, :auth_jwt_audience)

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
