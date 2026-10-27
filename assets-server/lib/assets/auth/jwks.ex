defmodule Assets.Auth.Jwks do
  @moduledoc """
  Assets-named JWKS cache — facade over `AlchemyAuthClient.Jwks` (x002f / P3-4).

  Guarantees: RS256 public keys from AUTH_JWKS_URL (or test install).
  Non-guarantees: revocation lists, user status DB lookups.
  """

  @server __MODULE__

  def child_spec(opts) do
    opts =
      opts
      |> Keyword.put(:otp_app, :assets)
      |> Keyword.put_new(:name, @server)

    %{
      id: @server,
      start: {AlchemyAuthClient.Jwks, :start_link, [opts]}
    }
  end

  @spec jwks() :: map()
  def jwks, do: AlchemyAuthClient.Jwks.jwks(@server)

  @spec signer_for_kid(String.t()) ::
          {:ok, Joken.Signer.t()} | {:error, :unknown_kid | :empty_jwks}
  def signer_for_kid(kid) when is_binary(kid),
    do: AlchemyAuthClient.Jwks.signer_for_kid(@server, kid)

  @spec refresh() :: :ok | {:error, term()}
  def refresh, do: AlchemyAuthClient.Jwks.refresh(@server)

  @doc false
  def install_test_jwks!(jwks, signers_by_kid) when is_map(jwks) and is_map(signers_by_kid) do
    AlchemyAuthClient.Jwks.install_test_jwks!(@server, jwks, signers_by_kid)
  end
end
