defmodule World.ManifestSeal do
  @moduledoc """
  Issues short-lived ManifestSeal JWTs (RS256) binding `(room_id, revision, manifest_blake3)`.

  Guarantees: same world JWKS as Room Tokens; distinct `aud` / `typ`.
  Non-guarantees: SceneManifest-driven content (P4); Presence / Frame world signatures.
  """

  alias World.Token.Keys

  @typ "manifest_seal"
  @aud "alchemy-manifest-seal"
  @blake3_hex_bytes 64

  @doc """
  Generate a seal JWT for a Manifest body hash.

  `manifest_blake3` must be lowercase hex (64 chars) of the canonical protobuf
  (with `world_seal` empty).
  """
  @spec generate(String.t(), non_neg_integer(), String.t(), String.t()) ::
          {:ok, String.t(), non_neg_integer()} | {:error, term()}
  def generate(room_id, revision, manifest_blake3, sub)
      when is_binary(room_id) and is_integer(revision) and revision >= 0 and
             is_binary(manifest_blake3) and
             is_binary(sub) do
    with :ok <- validate_room_id(room_id),
         :ok <- validate_blake3(manifest_blake3) do
      ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
      issuer = Application.fetch_env!(:world, :room_token_issuer)
      jti = uuid_v4()

      extra = %{
        "typ" => @typ,
        "sub" => sub,
        "room_id" => room_id,
        "revision" => revision,
        "manifest_blake3" => String.downcase(manifest_blake3),
        "jti" => jti
      }

      config =
        Joken.Config.default_claims(default_exp: ttl, iss: issuer, aud: @aud)
        |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
        |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
        |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &is_binary/1)
        |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)

      case Joken.generate_and_sign(config, extra, Keys.signer()) do
        {:ok, token, _claims} -> {:ok, token, ttl}
        {:error, reason} -> {:error, reason}
      end
    end
  end

  def generate(_, _, _, _), do: {:error, :invalid_args}

  @doc "Parse revision from API body (integer or decimal string)."
  @spec parse_revision(term()) :: {:ok, non_neg_integer()} | {:error, :invalid_revision}
  def parse_revision(value) when is_integer(value) and value >= 0, do: {:ok, value}

  def parse_revision(value) when is_binary(value) do
    case Integer.parse(value) do
      {n, ""} when n >= 0 -> {:ok, n}
      _ -> {:error, :invalid_revision}
    end
  end

  def parse_revision(_), do: {:error, :invalid_revision}

  @doc "Normalize and validate blake3 hex from API body."
  @spec parse_blake3(term()) :: {:ok, String.t()} | {:error, :invalid_manifest_blake3}
  def parse_blake3(value) when is_binary(value) do
    down = String.downcase(value)

    if valid_blake3_hex?(down) do
      {:ok, down}
    else
      {:error, :invalid_manifest_blake3}
    end
  end

  def parse_blake3(_), do: {:error, :invalid_manifest_blake3}

  @doc "Audience claim for ManifestSeal JWTs."
  def audience, do: @aud

  @doc "typ claim value."
  def typ, do: @typ

  defp validate_blake3(hex) do
    if valid_blake3_hex?(String.downcase(hex)) do
      :ok
    else
      {:error, :invalid_manifest_blake3}
    end
  end

  defp valid_blake3_hex?(hex) do
    byte_size(hex) == @blake3_hex_bytes and Regex.match?(~r/^[0-9a-f]+$/, hex)
  end

  defp validate_room_id(value) do
    if is_binary(value) and value != "" and not String.contains?(value, "/") and
         byte_size(value) <= 64 do
      :ok
    else
      {:error, :invalid_room_id}
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
