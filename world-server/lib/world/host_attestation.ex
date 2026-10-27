defmodule World.HostAttestation do
  @moduledoc """
  Issues short-lived HostAttestation JWTs (RS256) proving `(room_id, host_pub)`.

  Guarantees: same world JWKS as Room Tokens; distinct `aud` / `typ`.
  Non-guarantees: Manifest body integrity (see ManifestSeal / x0026); Presence world signatures;
  SceneManifest-driven content (P4).
  """

  alias World.Token.Keys

  @typ "host_attestation"
  @aud "alchemy-host-attestation"
  @host_pub_bytes 32

  @doc """
  Generate an attestation JWT for a 32-byte Ed25519 `host_pub`.

  `host_pub` must already be raw binary (not base64).
  """
  @spec generate(String.t(), binary(), String.t()) ::
          {:ok, String.t(), non_neg_integer()} | {:error, term()}
  def generate(room_id, host_pub, sub)
      when is_binary(room_id) and is_binary(host_pub) and is_binary(sub) do
    with :ok <- validate_room_id(room_id),
         :ok <- validate_host_pub(host_pub) do
      ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
      issuer = Application.fetch_env!(:world, :room_token_issuer)
      jti = uuid_v4()

      extra = %{
        "typ" => @typ,
        "sub" => sub,
        "room_id" => room_id,
        "host_pub" => Base.url_encode64(host_pub, padding: false),
        "jti" => jti
      }

      config =
        Joken.Config.default_claims(default_exp: ttl, iss: issuer, aud: @aud)
        |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
        |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("host_pub", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

      case Joken.generate_and_sign(config, extra, Keys.signer()) do
        {:ok, token, _claims} -> {:ok, token, ttl}
        {:error, reason} -> {:error, reason}
      end
    end
  end

  def generate(_, _, _), do: {:error, :invalid_args}

  @doc "Decode base64url host_pub from API body into 32 raw bytes."
  @spec decode_host_pub(String.t()) :: {:ok, binary()} | {:error, :invalid_host_pub}
  def decode_host_pub(encoded) when is_binary(encoded) do
    case Base.url_decode64(encoded, padding: false) do
      {:ok, bin} when byte_size(bin) == @host_pub_bytes -> {:ok, bin}
      _ -> {:error, :invalid_host_pub}
    end
  end

  def decode_host_pub(_), do: {:error, :invalid_host_pub}

  @doc "Audience claim for HostAttestation JWTs."
  def audience, do: @aud

  @doc "typ claim value."
  def typ, do: @typ

  defp validate_host_pub(bin) when byte_size(bin) == @host_pub_bytes, do: :ok
  defp validate_host_pub(_), do: {:error, :invalid_host_pub}

  defp validate_room_id(value), do: World.RoomId.validate(value)

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
