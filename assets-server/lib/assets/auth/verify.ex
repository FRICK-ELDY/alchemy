defmodule Assets.Auth.Verify do
  @moduledoc """
  Verifies auth-server user access JWTs (RS256) via cached JWKS.
  """

  @spec verify(String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token) do
    AlchemyAuthClient.Verify.verify(token, otp_app: :assets, jwks: Assets.Auth.Jwks)
  end
end
