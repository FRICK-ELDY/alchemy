defmodule Assets.PlaceHashCache do
  @moduledoc """
  Short-lived cache of place scene blake3s for PlaceRead (x006b).

  Guarantees: key = `(room_id, revision, manifest_blake3)`; TTL defaults to the
  world JWT lifetime (300s, overridable via `:place_hashes_ttl_ms` / env, never
  below that floor when world uses the same); miss may fetch place_hashes with
  PlaceRead Bearer; response identity must match claims.
  Non-guarantees: durable storage; federation discovery; surviving world restart
  without re-seal (world returns 409 → assets 403 until new seal).
  """

  use GenServer

  @table __MODULE__
  # Match world `room_token_ttl_seconds` (300) — JWT lifetime floor.
  @jwt_ttl_floor_ms 300_000
  @max_entries 2_048

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc "Test/helper: seed cache without HTTP."
  @spec put(String.t(), non_neg_integer(), String.t(), [String.t()]) :: :ok
  def put(room_id, revision, manifest_blake3, blake3s)
      when is_binary(room_id) and is_integer(revision) and is_binary(manifest_blake3) and
             is_list(blake3s) do
    prune_if_needed()

    true =
      :ets.insert(
        @table,
        {key(room_id, revision, manifest_blake3),
         %{blake3s: Enum.uniq(blake3s), expires_at_ms: now_ms() + ttl_ms()}}
      )

    :ok
  end

  @doc "Resolve scene blake3s for verified PlaceRead claims."
  @spec resolve(String.t(), map()) :: {:ok, [String.t()]} | {:error, term()}
  def resolve(place_read_token, claims)
      when is_binary(place_read_token) and is_map(claims) do
    room_id = claims["room_id"]
    revision = claims["revision"]
    manifest_blake3 = claims["manifest_blake3"]
    cache_key = key(room_id, revision, manifest_blake3)

    case :ets.lookup(@table, cache_key) do
      [{^cache_key, %{blake3s: blake3s, expires_at_ms: exp}}] ->
        if now_ms() < exp do
          {:ok, blake3s}
        else
          :ets.delete(@table, cache_key)
          fetch_and_cache(place_read_token, room_id, revision, manifest_blake3)
        end

      [] ->
        fetch_and_cache(place_read_token, room_id, revision, manifest_blake3)
    end
  end

  @impl true
  def init(_opts) do
    :ets.new(@table, [
      :named_table,
      :public,
      :set,
      read_concurrency: true,
      write_concurrency: true
    ])

    {:ok, %{}}
  end

  defp fetch_and_cache(token, room_id, revision, manifest_blake3) do
    with {:ok, base} <- world_base_url(),
         url <- String.trim_trailing(base, "/") <> "/api/v1/rooms/#{room_id}/place_hashes",
         {:ok, body} <- http_place_hashes(url, token),
         :ok <- match_identity(body, room_id, revision, manifest_blake3),
         {:ok, blake3s} <- extract_blake3s(body) do
      put(room_id, revision, manifest_blake3, blake3s)
      {:ok, blake3s}
    end
  end

  defp world_base_url do
    case Application.get_env(:assets, :world_base_url) do
      base when is_binary(base) and base != "" -> {:ok, base}
      _ -> {:error, :world_base_unconfigured}
    end
  end

  defp http_place_hashes(url, token) do
    case Req.get(url,
           headers: [{"authorization", "Bearer #{token}"}],
           receive_timeout: 3_000,
           retry: false
         ) do
      {:ok, %{status: 200, body: body}} when is_map(body) -> {:ok, body}
      {:ok, %{status: status}} -> {:error, {:http_status, status}}
      {:error, reason} -> {:error, reason}
    end
  end

  defp match_identity(body, room_id, revision, manifest_blake3) do
    body_room = Map.get(body, "room_id")
    body_rev = Map.get(body, "revision")
    body_hash = Map.get(body, "manifest_blake3")

    cond do
      body_room != room_id ->
        {:error, :place_hashes_identity_mismatch}

      body_rev != revision ->
        {:error, :place_hashes_identity_mismatch}

      not is_binary(body_hash) ->
        {:error, :place_hashes_identity_mismatch}

      String.downcase(body_hash) != String.downcase(manifest_blake3) ->
        {:error, :place_hashes_identity_mismatch}

      true ->
        :ok
    end
  end

  defp extract_blake3s(%{"blake3s" => blake3s}) when is_list(blake3s) do
    cleaned =
      blake3s
      |> Enum.filter(&(is_binary(&1) and Regex.match?(~r/^[0-9a-f]{64}$/, String.downcase(&1))))
      |> Enum.map(&String.downcase/1)
      |> Enum.uniq()

    {:ok, cleaned}
  end

  defp extract_blake3s(_), do: {:error, :invalid_place_hashes_body}

  defp ttl_ms do
    floor = Application.get_env(:assets, :place_hashes_ttl_floor_ms, @jwt_ttl_floor_ms)

    case Application.get_env(:assets, :place_hashes_ttl_ms) do
      ms when is_integer(ms) and ms > 0 -> max(ms, floor)
      _ -> floor
    end
  end

  defp prune_if_needed do
    if :ets.info(@table, :size) >= @max_entries do
      now = now_ms()
      delete_expired(now)

      if :ets.info(@table, :size) >= @max_entries do
        delete_oldest(:ets.info(@table, :size) - @max_entries + 1)
      end
    end
  end

  defp delete_expired(now) do
    :ets.foldl(
      fn
        {k, %{expires_at_ms: exp}}, acc when exp <= now ->
          :ets.delete(@table, k)
          acc

        _, acc ->
          acc
      end,
      :ok,
      @table
    )
  end

  defp delete_oldest(count) when count <= 0, do: :ok

  defp delete_oldest(count) do
    :ets.foldl(
      fn
        {key, %{expires_at_ms: exp}}, acc -> [{key, exp} | acc]
        _, acc -> acc
      end,
      [],
      @table
    )
    |> Enum.sort_by(fn {_k, exp} -> exp end)
    |> Enum.take(count)
    |> Enum.each(fn {key, _} -> :ets.delete(@table, key) end)
  end

  defp key(room_id, revision, manifest_blake3),
    do: {room_id, revision, String.downcase(manifest_blake3)}

  defp now_ms, do: System.monotonic_time(:millisecond)
end
