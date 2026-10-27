defmodule AlchemyAuthClient.VerifyTest do
  use ExUnit.Case, async: false

  alias AlchemyAuthClient.{Jwks, Verify}

  @otp_app :alchemy_auth_client
  @server __MODULE__.JwksServer
  @issuer "alchemy-auth-test"
  @audience "alchemy-platform-test"

  setup do
    previous = %{
      url: Application.get_env(@otp_app, :auth_jwks_url),
      fetch: Application.get_env(@otp_app, :auth_jwks_fetch_on_start),
      iss: Application.get_env(@otp_app, :auth_jwt_issuer),
      aud: Application.get_env(@otp_app, :auth_jwt_audience)
    }

    Application.put_env(@otp_app, :auth_jwks_url, "http://127.0.0.1:9/unused")
    Application.put_env(@otp_app, :auth_jwks_fetch_on_start, false)
    Application.put_env(@otp_app, :auth_jwt_issuer, @issuer)
    Application.put_env(@otp_app, :auth_jwt_audience, @audience)

    on_exit(fn ->
      restore_env(:auth_jwks_url, previous.url)
      restore_env(:auth_jwks_fetch_on_start, previous.fetch)
      restore_env(:auth_jwt_issuer, previous.iss)
      restore_env(:auth_jwt_audience, previous.aud)
      if Process.whereis(@server), do: GenServer.stop(@server)
    end)

    {:ok, _} = Jwks.start_link(otp_app: @otp_app, name: @server, fetch_cooldown_ms: 1_000)

    {public_key, sign_signer, kid} = rsa_pair()

    verify_signer =
      Joken.Signer.create("RS256", %{
        "kty" => "RSA",
        "n" => public_key["n"],
        "e" => public_key["e"],
        "kid" => kid
      })

    :ok = Jwks.install_test_jwks!(@server, %{"keys" => [public_key]}, %{kid => verify_signer})

    %{sign_signer: sign_signer, kid: kid}
  end

  test "verify accepts active token with matching iss/aud", %{sign_signer: signer} do
    token = sign_token(signer, status: "active")
    assert {:ok, claims} = Verify.verify(token, otp_app: @otp_app, jwks: @server)
    assert claims["sub"] == "user-1"
    assert claims["status"] == "active"
  end

  test "verify rejects wrong issuer", %{sign_signer: signer} do
    token = sign_token(signer, status: "active", iss: "other-iss")
    assert {:error, _} = Verify.verify(token, otp_app: @otp_app, jwks: @server)
  end

  test "verify rejects wrong audience", %{sign_signer: signer} do
    token = sign_token(signer, status: "active", aud: "other-aud")
    assert {:error, _} = Verify.verify(token, otp_app: @otp_app, jwks: @server)
  end

  test "verify rejects inactive status", %{sign_signer: signer} do
    token = sign_token(signer, status: "disabled")
    assert {:error, :inactive_user} = Verify.verify(token, otp_app: @otp_app, jwks: @server)
  end

  test "verify allows claims without status field", %{sign_signer: signer} do
    token = sign_token(signer, status: :omit)
    assert {:ok, claims} = Verify.verify(token, otp_app: @otp_app, jwks: @server)
    refute Map.has_key?(claims, "status")
  end

  test "verify rejects missing token" do
    assert {:error, :missing_token} = Verify.verify(nil, otp_app: @otp_app, jwks: @server)
    assert {:error, :missing_token} = Verify.verify("", otp_app: @otp_app, jwks: @server)
  end

  defp restore_env(key, nil), do: Application.delete_env(@otp_app, key)
  defp restore_env(key, value), do: Application.put_env(@otp_app, key, value)

  defp rsa_pair do
    jwk = JOSE.JWK.generate_key({:rsa, 2048})
    public = JOSE.JWK.to_public(jwk)
    {_fields, map} = JOSE.JWK.to_map(public)
    kid = "verify-kid-" <> Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)
    {_type, private_pem} = JOSE.JWK.to_pem(jwk)

    public_key =
      map
      |> Map.put("kid", kid)
      |> Map.put("use", "sig")
      |> Map.put("alg", "RS256")

    sign_signer = Joken.Signer.create("RS256", %{"pem" => private_pem}, %{"kid" => kid})
    {public_key, sign_signer, kid}
  end

  defp sign_token(signer, opts) do
    status = Keyword.get(opts, :status, "active")
    iss = Keyword.get(opts, :iss, @issuer)
    aud = Keyword.get(opts, :aud, @audience)

    claims =
      %{
        "sub" => "user-1",
        "jti" => Base.encode16(:crypto.strong_rand_bytes(8), case: :lower),
        "iss" => iss,
        "aud" => aud
      }
      |> then(fn c ->
        case status do
          :omit -> c
          other -> Map.put(c, "status", other)
        end
      end)

    config =
      Joken.Config.default_claims(default_exp: 900, skip: [:iss, :aud])
      |> Joken.Config.add_claim("sub", fn -> "user-1" end, &is_binary/1)
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

    {:ok, token, _} = Joken.generate_and_sign(config, claims, signer)
    token
  end
end
