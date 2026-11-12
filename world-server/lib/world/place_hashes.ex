defmodule World.PlaceHashes do
  @moduledoc """
  Seal-bound scene blake3 cache for PlaceRead (x006b).

  Guarantees: allowlist is captured **at ManifestSeal time** and keyed by
  `(room_id, revision, manifest_blake3)`; TTL is at least the Room/ManifestSeal/
  PlaceRead JWT lifetime (`room_token_ttl_seconds`); `resolve/3` never reloads
  live scene into an existing seal key.
  Non-guarantees: durable storage (process restart clears cache → re-seal required);
  cross-node replication; scene edits without re-seal.
  """

  use GenServer

  alias World.Rooms

  @table __MODULE__
  @max_entries 2_048

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc "Extract sorted unique non-empty blake3 hex strings from a scene map."
  @spec from_scene(map()) :: [String.t()]
  def from_scene(%{"entries" => entries}) when is_list(entries) do
    entries
    |> Enum.flat_map(fn
      %{"blake3" => blake3} when is_binary(blake3) ->
        down = String.downcase(String.trim(blake3))

        if down != "" and Regex.match?(~r/^[0-9a-f]{64}$/, down) do
          [down]
        else
          []
        end

      _ ->
        []
    end)
    |> Enum.uniq()
    |> Enum.sort()
  end

  def from_scene(_), do: []

  @doc "Load composed scene (x009e), require matching packed revision, return blake3 list."
  @spec scene_blake3s(String.t(), non_neg_integer()) ::
          {:ok, [String.t()]} | {:error, atom()}
  def scene_blake3s(room_id, revision)
      when is_binary(room_id) and is_integer(revision) and revision >= 0 do
    with {:ok, scene} <- Rooms.composed_scene(room_id),
         {:ok, scene_rev} <- scene_revision(scene),
         :ok <- match_revision(scene_rev, revision) do
      {:ok, from_scene(scene)}
    end
  end

  def scene_blake3s(_, _), do: {:error, :invalid_args}

  @doc """
  Capture composed-scene blake3s for a just-issued ManifestSeal identity (x009e).

  Call only from the seal path.
  """
  @spec put_for_seal(String.t(), non_neg_integer(), String.t()) ::
          {:ok, [String.t()]} | {:error, atom()}
  def put_for_seal(room_id, revision, manifest_blake3)
      when is_binary(room_id) and is_integer(revision) and is_binary(manifest_blake3) do
    with {:ok, blake3s} <- scene_blake3s(room_id, revision) do
      put(room_id, revision, manifest_blake3, blake3s)
      {:ok, blake3s}
    end
  end

  @doc "Remember blake3s for a sealed place identity (TTL ≥ JWT lifetime)."
  @spec put(String.t(), non_neg_integer(), String.t(), [String.t()]) :: :ok
  def put(room_id, revision, manifest_blake3, blake3s)
      when is_binary(room_id) and is_integer(revision) and is_binary(manifest_blake3) and
             is_list(blake3s) do
    prune_if_needed()

    true =
      :ets.insert(@table, {
        cache_key(room_id, revision, manifest_blake3),
        %{
          blake3s: Enum.sort(Enum.uniq(blake3s)),
          expires_at_ms: now_ms() + ttl_ms()
        }
      })

    :ok
  end

  @doc "Lookup non-expired cached blake3s for a sealed place identity."
  @spec get(String.t(), non_neg_integer(), String.t()) ::
          {:ok, [String.t()]} | :miss
  def get(room_id, revision, manifest_blake3)
      when is_binary(room_id) and is_integer(revision) and is_binary(manifest_blake3) do
    key = cache_key(room_id, revision, manifest_blake3)

    case :ets.lookup(@table, key) do
      [{^key, %{blake3s: blake3s, expires_at_ms: exp}}] ->
        if now_ms() < exp do
          {:ok, blake3s}
        else
          :ets.delete(@table, key)
          :miss
        end

      [] ->
        :miss
    end
  end

  @doc """
  Resolve blake3s for PlaceRead / place_hashes.

  Cache miss does **not** reload live scene (seal key must be populated at seal time;
  after process restart, host must re-seal).
  """
  @spec resolve(String.t(), non_neg_integer(), String.t()) ::
          {:ok, [String.t()]} | {:error, atom()}
  def resolve(room_id, revision, manifest_blake3) do
    case get(room_id, revision, manifest_blake3) do
      {:ok, _} = hit -> hit
      :miss -> {:error, :place_hashes_unavailable}
    end
  end

  @doc """
  Align seal-key allowlist expiry to ManifestSeal absolute unix `exp`.

  Sets monotonic deadline to ~`seal_exp` wall time (never extends past that endpoint,
  even if `:place_hashes_ttl_ms` was longer at put).
  """
  @spec touch_until(String.t(), non_neg_integer(), String.t(), pos_integer()) ::
          :ok | :miss | {:error, :seal_expired}
  def touch_until(room_id, revision, manifest_blake3, seal_exp)
      when is_binary(room_id) and is_integer(revision) and is_binary(manifest_blake3) and
             is_integer(seal_exp) do
    remaining_ms = (seal_exp - System.system_time(:second)) * 1_000

    if remaining_ms < 1_000 do
      {:error, :seal_expired}
    else
      key = cache_key(room_id, revision, manifest_blake3)
      # Cap to seal endpoint (do not keep a longer put TTL beyond seal.exp).
      new_exp = now_ms() + remaining_ms

      case :ets.lookup(@table, key) do
        [{^key, %{blake3s: blake3s}}] ->
          true = :ets.insert(@table, {key, %{blake3s: blake3s, expires_at_ms: new_exp}})
          :ok

        [] ->
          :miss
      end
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

  # Prefer explicit :place_hashes_ttl_ms, else room_token_ttl_seconds (JWT lifetime).
  defp ttl_ms do
    case Application.get_env(:world, :place_hashes_ttl_ms) do
      ms when is_integer(ms) and ms > 0 ->
        max(ms, jwt_ttl_ms())

      _ ->
        jwt_ttl_ms()
    end
  end

  defp jwt_ttl_ms do
    Application.fetch_env!(:world, :room_token_ttl_seconds) * 1_000
  end

  defp prune_if_needed do
    if :ets.info(@table, :size) >= @max_entries do
      delete_expired(now_ms())
      size_after = :ets.info(@table, :size)

      if size_after >= @max_entries do
        delete_oldest(size_after - @max_entries + 1)
      end
    end
  end

  defp delete_expired(now) do
    :ets.foldl(
      fn
        {key, %{expires_at_ms: exp}}, acc when exp <= now ->
          :ets.delete(@table, key)
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
    oldest =
      :ets.foldl(
        fn
          {key, %{expires_at_ms: exp}}, acc ->
            [{key, exp} | acc]

          _, acc ->
            acc
        end,
        [],
        @table
      )
      |> Enum.sort_by(fn {_key, exp} -> exp end)
      |> Enum.take(count)

    Enum.each(oldest, fn {key, _} -> :ets.delete(@table, key) end)
  end

  defp cache_key(room_id, revision, manifest_blake3),
    do: {room_id, revision, String.downcase(manifest_blake3)}

  defp scene_revision(%{"revision" => rev}) when is_integer(rev) and rev >= 0, do: {:ok, rev}

  defp scene_revision(%{"revision" => rev}) when is_binary(rev) do
    case Integer.parse(rev) do
      {n, ""} when n >= 0 -> {:ok, n}
      _ -> {:error, :invalid_scene_revision}
    end
  end

  defp scene_revision(_), do: {:error, :invalid_scene_revision}

  defp match_revision(a, b) when a == b, do: :ok
  defp match_revision(_, _), do: {:error, :revision_mismatch}

  defp now_ms, do: System.monotonic_time(:millisecond)
end
