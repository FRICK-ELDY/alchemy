defmodule World.PlaceRead do
  @moduledoc """
  Issues short-lived PlaceRead JWTs (RS256) for place-scoped blob GET (x006b).

  Guarantees: same world JWKS as Room Tokens; distinct `aud` / `typ`; seal-linked
  `(room_id, revision, host_generation, manifest_blake3)`; **PlaceRead `exp` equals
  the verified ManifestSeal absolute unix `exp`** (same endpoint as seal / allowlist).
  Non-guarantees: `extra_blake3` authorization (disabled until take-out binds ownership /
  in-room presence); embedding full scene hash lists in the JWT; federated auto-mirror;
  immediate revoke on leave (TTL only); surviving world restart without re-seal.
  """

  alias World.Token.Keys

  @typ "place_read"
  @aud "alchemy-assets-place-read"
  @blake3_hex_bytes 64

  @doc "Audience claim for PlaceRead JWTs."
  def audience, do: @aud

  @doc "typ claim value."
  def typ, do: @typ

  @doc """
  Absolute unix `exp` from verified ManifestSeal claims.

  PlaceRead must use this same endpoint (not `now + remaining`).
  """
  @spec seal_exp_from_claims(map()) ::
          {:ok, pos_integer()} | {:error, :seal_expired | :invalid_seal_exp}
  def seal_exp_from_claims(%{"exp" => exp}) when is_integer(exp) and exp > 0 do
    if exp > System.system_time(:second) do
      {:ok, exp}
    else
      {:error, :seal_expired}
    end
  end

  def seal_exp_from_claims(_), do: {:error, :invalid_seal_exp}

  @doc "Seconds until `seal_exp` (for `expires_in` response / cache touch)."
  @spec seconds_until(pos_integer()) :: {:ok, pos_integer()} | {:error, :seal_expired}
  def seconds_until(seal_exp) when is_integer(seal_exp) do
    rem = seal_exp - System.system_time(:second)

    if rem >= 1 do
      {:ok, rem}
    else
      {:error, :seal_expired}
    end
  end

  @doc """
  Generate a PlaceRead JWT whose `exp` claim is exactly `seal_exp` (unix).

  `extra_blake3` must be `[]`.
  """
  @spec generate(
          String.t(),
          String.t(),
          non_neg_integer(),
          pos_integer(),
          String.t(),
          [String.t()],
          pos_integer()
        ) ::
          {:ok, String.t(), pos_integer()} | {:error, term()}
  def generate(room_id, sub, revision, host_generation, manifest_blake3, extra_blake3, seal_exp)
      when is_binary(room_id) and is_binary(sub) and is_integer(revision) and revision >= 0 and
             is_integer(host_generation) and host_generation >= 1 and
             is_binary(manifest_blake3) and is_list(extra_blake3) and
             is_integer(seal_exp) do
    with :ok <- World.RoomId.validate(room_id),
         :ok <- validate_blake3(manifest_blake3),
         :ok <- require_empty_extra(extra_blake3),
         {:ok, expires_in} <- seconds_until(seal_exp),
         claims <-
           build_claims(sub, room_id, revision, host_generation, manifest_blake3, seal_exp),
         {:ok, token} <- sign_place_read(claims) do
      {:ok, token, expires_in}
    end
  end

  def generate(_, _, _, _, _, _, _), do: {:error, :invalid_args}

  defp build_claims(sub, room_id, revision, host_generation, manifest_blake3, seal_exp) do
    now = System.system_time(:second)

    %{
      "typ" => @typ,
      "sub" => sub,
      "room_id" => room_id,
      "revision" => revision,
      "host_generation" => host_generation,
      "manifest_blake3" => String.downcase(manifest_blake3),
      "extra_blake3" => [],
      "jti" => uuid_v4(),
      "iat" => now,
      "nbf" => now,
      "exp" => seal_exp
    }
  end

  # Absolute seal.exp is the SSoT endpoint — skip default exp/iat/nbf.
  defp sign_place_read(claims) do
    issuer = Application.fetch_env!(:world, :room_token_issuer)

    config =
      Joken.Config.default_claims(skip: [:exp, :iat, :nbf], iss: issuer, aud: @aud)
      |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
      |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("host_generation", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("extra_blake3", fn -> [] end, &(&1 == []))
      |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
      |> Joken.Config.add_claim("iat", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("nbf", fn -> nil end, &is_integer/1)
      |> Joken.Config.add_claim("exp", fn -> nil end, &is_integer/1)

    case Joken.generate_and_sign(config, claims, Keys.signer()) do
      {:ok, token, _claims} -> {:ok, token}
      {:error, reason} -> {:error, reason}
    end
  end

  @doc "Verify a PlaceRead JWT with world Room Token keys."
  @spec verify(String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token) when is_binary(token) do
    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- Keys.signer_for_kid(kid),
         {:ok, claims} <- Joken.verify_and_validate(verify_config(), token, signer),
         :ok <- require_empty_extra(Map.get(claims, "extra_blake3", [])) do
      {:ok, claims}
    else
      {:error, reason} -> {:error, reason}
      other -> {:error, {:unexpected, other}}
    end
  end

  def verify(_), do: {:error, :invalid_token}

  @doc """
  Normalize API `extra_blake3` body field.

  x006b hardening: only omitted / nil / [] accepted. Non-empty → `:extra_blake3_disabled`.
  """
  @spec parse_extra_blake3(term()) :: {:ok, [String.t()]} | {:error, atom()}
  def parse_extra_blake3(nil), do: {:ok, []}
  def parse_extra_blake3([]), do: {:ok, []}
  def parse_extra_blake3(list) when is_list(list), do: {:error, :extra_blake3_disabled}
  def parse_extra_blake3(_), do: {:error, :invalid_extra_blake3}

  defp require_empty_extra([]), do: :ok
  defp require_empty_extra(_), do: {:error, :extra_blake3_disabled}

  defp verify_config do
    ttl = Application.fetch_env!(:world, :room_token_ttl_seconds)
    issuer = Application.fetch_env!(:world, :room_token_issuer)

    Joken.Config.default_claims(default_exp: ttl, iss: issuer, aud: @aud)
    |> Joken.Config.add_claim("typ", fn -> nil end, &(&1 == @typ))
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
    |> Joken.Config.add_claim("host_generation", fn -> nil end, &is_integer/1)
    |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("extra_blake3", fn -> [] end, &(&1 == []))
    |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
  end

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

  defp uuid_v4 do
    <<a::32, b::16, c::16, d::16, e::48>> = :crypto.strong_rand_bytes(16)
    c = Bitwise.bor(Bitwise.band(c, 0x0FFF), 0x4000)
    d = Bitwise.bor(Bitwise.band(d, 0x3FFF), 0x8000)

    :io_lib.format("~8.16.0b-~4.16.0b-~4.16.0b-~4.16.0b-~12.16.0b", [a, b, c, d, e])
    |> IO.iodata_to_binary()
  end
end
