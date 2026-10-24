defmodule Assets.TestAuthHelpers do
  @moduledoc """
  Installs a local RSA keypair as fake auth-server JWKS for controller tests.
  """

  alias Assets.Auth.Jwks

  @issuer "alchemy-auth"
  @audience "alchemy-platform"

  def install_test_auth! do
    jwk = JOSE.JWK.generate_key({:rsa, 2048})
    public_jwk = JOSE.JWK.to_public(jwk)
    kid = thumbprint_kid(public_jwk)
    {_fields, jwk_map} = JOSE.JWK.to_map(public_jwk)
    {_type, private_pem} = JOSE.JWK.to_pem(jwk)

    public_key_map =
      jwk_map
      |> Map.put("kid", kid)
      |> Map.put("use", "sig")
      |> Map.put("alg", "RS256")

    jwks = %{"keys" => [public_key_map]}

    verify_signer =
      Joken.Signer.create("RS256", %{
        "kty" => "RSA",
        "n" => public_key_map["n"],
        "e" => public_key_map["e"],
        "kid" => kid
      })

    sign_signer = Joken.Signer.create("RS256", %{"pem" => private_pem}, %{"kid" => kid})
    Jwks.install_test_jwks!(jwks, %{kid => verify_signer})

    %{kid: kid, signer: sign_signer}
  end

  def auth_token(signer, sub \\ "11111111-1111-1111-1111-111111111111") do
    claims = %{
      "sub" => sub,
      "status" => "active",
      "jti" => uuid_v4(),
      "iss" => @issuer,
      "aud" => @audience
    }

    config =
      Joken.Config.default_claims(default_exp: 900, iss: @issuer, aud: @audience)
      |> Joken.Config.add_claim("sub", fn -> sub end, &is_binary/1)
      |> Joken.Config.add_claim("status", fn -> "active" end, &(&1 == "active"))
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

    {:ok, token, _} = Joken.generate_and_sign(config, claims, signer)
    token
  end

  def put_auth(conn, signer, sub \\ "11111111-1111-1111-1111-111111111111") do
    Plug.Conn.put_req_header(conn, "authorization", "Bearer " <> auth_token(signer, sub))
  end

  defp thumbprint_kid(jwk) do
    case JOSE.JWK.thumbprint(jwk) do
      {:kid, kid} -> kid
      kid when is_binary(kid) -> kid
    end
  end

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
