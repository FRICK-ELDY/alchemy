defmodule Assets.TestPlaceReadHelpers do
  @moduledoc false

  alias Assets.PlaceHashCache
  alias Assets.World.Jwks

  @issuer "alchemy-world"
  @aud "alchemy-assets-place-read"
  @typ "place_read"

  def install_test_world_jwks! do
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

  def place_read_token(signer, opts) do
    sub = Keyword.fetch!(opts, :sub)
    room_id = Keyword.get(opts, :room_id, "local")
    revision = Keyword.get(opts, :revision, 1)
    host_generation = Keyword.get(opts, :host_generation, 1)
    manifest_blake3 = Keyword.get(opts, :manifest_blake3, String.duplicate("ab", 32))
    extra = Keyword.get(opts, :extra_blake3, [])

    claims = %{
      "typ" => @typ,
      "sub" => sub,
      "room_id" => room_id,
      "revision" => revision,
      "host_generation" => host_generation,
      "manifest_blake3" => manifest_blake3,
      "extra_blake3" => extra,
      "jti" => uuid_v4(),
      "iss" => @issuer,
      "aud" => @aud
    }

    config =
      Joken.Config.default_claims(default_exp: 300, iss: @issuer, aud: @aud)
      |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
      |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("host_generation", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("extra_blake3", fn -> [] end, &is_list/1)
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

    {:ok, token, _} = Joken.generate_and_sign(config, claims, signer)
    token
  end

  def seed_scene_hashes(room_id, revision, manifest_blake3, blake3s) do
    PlaceHashCache.put(room_id, revision, manifest_blake3, blake3s)
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
