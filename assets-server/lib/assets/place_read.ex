defmodule Assets.PlaceRead do
  @moduledoc """
  Verifies world-issued PlaceRead JWTs and resolves allowed blake3s (x006b).

  Guarantees: `typ=place_read` / `aud=alchemy-assets-place-read`; `sub` must match
  the assets auth JWT subject; allowlist = seal-time scene hashes only
  (`extra_blake3` must be empty — disabled until take-out ownership binding).
  Non-guarantees: issuing PlaceRead (world only); GET default capability flip (案 B).
  """

  alias Assets.PlaceHashCache
  alias Assets.World.Jwks

  @typ "place_read"
  @aud "alchemy-assets-place-read"
  @blake3_hex_bytes 64
  @verify_cache_table __MODULE__.VerifyCache
  @default_verify_ttl_ms 5_000
  @default_verify_cache_max 4_096

  @doc false
  def ensure_verify_cache! do
    case :ets.whereis(@verify_cache_table) do
      :undefined ->
        :ets.new(@verify_cache_table, [
          :named_table,
          :public,
          :set,
          read_concurrency: true,
          write_concurrency: true
        ])

      _ ->
        @verify_cache_table
    end
  end

  @doc "Verify PlaceRead JWT and ensure claims.sub == auth_sub (short-memoized)."
  @spec verify(String.t(), String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token, auth_sub)
      when is_binary(token) and is_binary(auth_sub) do
    ensure_verify_cache!()
    cache_key = {:v1, :crypto.hash(:sha256, token), auth_sub}

    case :ets.lookup(@verify_cache_table, cache_key) do
      [{^cache_key, claims, exp}] when is_map(claims) ->
        if System.monotonic_time(:millisecond) < exp do
          {:ok, claims}
        else
          :ets.delete(@verify_cache_table, cache_key)
          verify_uncached(token, auth_sub, cache_key)
        end

      _ ->
        verify_uncached(token, auth_sub, cache_key)
    end
  end

  def verify(_, _), do: {:error, :invalid_args}

  @doc """
  True when PlaceRead authorizes GET of `blake3` for `auth_sub`.

  Scene hashes only (no `extra_blake3` bypass).
  """
  @spec allows_blake3?(String.t(), String.t(), String.t()) :: boolean()
  def allows_blake3?(token, auth_sub, blake3)
      when is_binary(token) and is_binary(auth_sub) and is_binary(blake3) do
    down = String.downcase(blake3)

    case verify(token, auth_sub) do
      {:ok, claims} ->
        case PlaceHashCache.resolve(token, claims) do
          {:ok, scene_hashes} -> down in scene_hashes
          {:error, _} -> false
        end

      {:error, _} ->
        false
    end
  end

  def allows_blake3?(_, _, _), do: false

  defp verify_uncached(token, auth_sub, cache_key) do
    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- Jwks.signer_for_kid(kid),
         {:ok, claims} <- Joken.verify_and_validate(verify_config(), token, signer),
         :ok <- match_sub(claims, auth_sub),
         :ok <- require_empty_extra(Map.get(claims, "extra_blake3", [])) do
      ttl = Application.get_env(:assets, :place_read_verify_ttl_ms, @default_verify_ttl_ms)
      prune_verify_cache!()

      :ets.insert(
        @verify_cache_table,
        {cache_key, claims, System.monotonic_time(:millisecond) + ttl}
      )

      {:ok, claims}
    else
      {:error, reason} -> {:error, reason}
      other -> {:error, {:unexpected, other}}
    end
  end

  defp match_sub(%{"sub" => sub}, auth_sub) when sub == auth_sub, do: :ok
  defp match_sub(_, _), do: {:error, :sub_mismatch}

  defp require_empty_extra([]), do: :ok
  defp require_empty_extra(_), do: {:error, :extra_blake3_disabled}

  defp verify_config do
    issuer = Application.get_env(:assets, :world_jwt_issuer, "alchemy-world")

    Joken.Config.default_claims(iss: issuer, aud: @aud)
    |> Joken.Config.add_claim("typ", fn -> nil end, &(&1 == @typ))
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("revision", fn -> nil end, &is_integer/1)
    |> Joken.Config.add_claim("host_generation", fn -> nil end, &is_integer/1)
    |> Joken.Config.add_claim("manifest_blake3", fn -> nil end, &valid_blake3?/1)
    |> Joken.Config.add_claim("extra_blake3", fn -> [] end, &valid_extra_claim?/1)
    |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
  end

  defp valid_extra_claim?([]), do: true
  defp valid_extra_claim?(_), do: false

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

  defp valid_blake3?(hex) when is_binary(hex) do
    byte_size(hex) == @blake3_hex_bytes and Regex.match?(~r/^[0-9a-f]+$/, hex)
  end

  defp valid_blake3?(_), do: false

  defp prune_verify_cache! do
    max = Application.get_env(:assets, :place_read_verify_cache_max, @default_verify_cache_max)
    now = System.monotonic_time(:millisecond)

    :ets.foldl(
      fn
        {key, _claims, exp}, acc when is_integer(exp) and exp <= now ->
          :ets.delete(@verify_cache_table, key)
          acc

        _, acc ->
          acc
      end,
      :ok,
      @verify_cache_table
    )

    size = :ets.info(@verify_cache_table, :size)

    if is_integer(size) and size >= max do
      :ets.foldl(
        fn
          {key, _claims, exp}, acc when is_integer(exp) -> [{key, exp} | acc]
          _, acc -> acc
        end,
        [],
        @verify_cache_table
      )
      |> Enum.sort_by(fn {_k, exp} -> exp end)
      |> Enum.take(size - max + 1)
      |> Enum.each(fn {key, _} -> :ets.delete(@verify_cache_table, key) end)
    end
  end
end
