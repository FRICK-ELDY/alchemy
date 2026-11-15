defmodule World.TestSpawnTicketHelpers do
  @moduledoc false

  alias World.Assets.Jwks

  @issuer "alchemy-assets"
  @aud "alchemy-world-spawn"
  @typ "spawn_ticket"

  def install_test_assets_jwks! do
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

    verify_signer =
      Joken.Signer.create("RS256", %{
        "kty" => "RSA",
        "n" => public_key_map["n"],
        "e" => public_key_map["e"],
        "kid" => kid
      })

    sign_signer = Joken.Signer.create("RS256", %{"pem" => private_pem}, %{"kid" => kid})
    Jwks.install_test_jwks!(%{"keys" => [public_key_map]}, %{kid => verify_signer})

    %{kid: kid, signer: sign_signer}
  end

  def spawn_ticket(signer, opts) do
    sub = Keyword.fetch!(opts, :sub)
    room_id = Keyword.fetch!(opts, :room_id)
    blake3s = opts |> Keyword.fetch!(:blake3s) |> Enum.map(&String.downcase/1)
    ttl = Keyword.get(opts, :ttl, 60)
    jti = Keyword.get(opts, :jti, uuid_v4())
    now = System.system_time(:second)
    exp = Keyword.get(opts, :exp, now + ttl)

    claims = %{
      "typ" => @typ,
      "sub" => sub,
      "room_id" => room_id,
      "blake3s" => blake3s,
      "jti" => jti,
      "iat" => now,
      "nbf" => now,
      "exp" => exp,
      "iss" => @issuer,
      "aud" => @aud
    }

    config =
      Joken.Config.default_claims(skip: [:exp, :iat, :nbf], iss: @issuer, aud: @aud)
      |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
      |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("blake3s", fn -> [] end, &is_list/1)
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("iat", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("nbf", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("exp", fn -> nil end, &is_integer/1)

    {:ok, token, _} = Joken.generate_and_sign(config, claims, signer)
    token
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
