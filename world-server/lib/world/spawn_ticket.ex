defmodule World.SpawnTicket do
  @moduledoc """
  Verifies assets-issued SpawnTicket JWTs and consumes `(jti, blake3)` once (x00b5).

  Guarantees: typ/aud/iss/exp/room_id/blake3∈list; participant `sub`; fail-closed 403 mapping.
  Claim policy: `(jti, blake3)` is consumed before append; stored value is ticket `exp`
  (prune only after the JWT itself can no longer verify). **Only**
  `:scene_revision_conflict` releases the claim (host CAS retry). Other append failures
  (`duplicate_blake3`, budget, storage, …) keep the claim burned — client must re-issue
  / re-take_out (intentional fail-closed; Prefab mid-fail needs a fresh ticket for that blake3).
  Non-guarantees: host-server honesty; persisting consume store across restart (re-issue tickets).
  """

  require Logger

  alias World.Assets.Jwks
  alias World.RoomParticipants

  @typ "spawn_ticket"
  @aud "alchemy-world-spawn"
  @blake3_hex_bytes 64
  @consume_table __MODULE__.Consumed
  @prune_interval_ms 5_000
  @prune_meta_key {__MODULE__, :last_prune_ms}

  @doc false
  def ensure_consume_table! do
    case :ets.whereis(@consume_table) do
      :undefined ->
        :ets.new(@consume_table, [
          :named_table,
          :public,
          :set,
          read_concurrency: true,
          write_concurrency: true
        ])

      _ ->
        @consume_table
    end
  end

  @doc """
  Verify ticket for `room_id` + `blake3`, require participant `sub`, then claim `(jti, blake3)`.

  On `:scene_revision_conflict` callers should `release_claim/2` so CAS retries can reuse the ticket.
  Other append failures must **not** release (fail-closed; re-take_out).
  """
  @spec authorize_and_claim(String.t() | nil, String.t(), String.t()) ::
          {:ok, map()} | {:error, :spawn_ticket_required | :spawn_ticket_invalid}
  def authorize_and_claim(nil, _room_id, _blake3), do: {:error, :spawn_ticket_required}
  def authorize_and_claim("", _room_id, _blake3), do: {:error, :spawn_ticket_required}

  def authorize_and_claim(token, room_id, blake3)
      when is_binary(token) and is_binary(room_id) and is_binary(blake3) do
    ensure_consume_table!()
    maybe_prune_expired_claims!()

    with {:ok, claims} <- verify(token),
         :ok <- match_room(claims, room_id),
         :ok <- blake3_allowed(claims, blake3),
         :ok <- require_participant(claims, room_id),
         :ok <- claim(claims["jti"], blake3, claims["exp"]) do
      {:ok, claims}
    else
      {:error, :spawn_ticket_required} ->
        {:error, :spawn_ticket_required}

      {:error, _reason} ->
        # Do not log reason codes (enumeration / slice: keep details suppressed).
        Logger.debug("spawn_ticket_invalid")
        {:error, :spawn_ticket_invalid}
    end
  end

  def authorize_and_claim(_, _, _), do: {:error, :spawn_ticket_invalid}

  @doc "Release a previously claimed `(jti, blake3)` (CAS conflict retry only)."
  @spec release_claim(String.t(), String.t()) :: :ok
  def release_claim(jti, blake3) when is_binary(jti) and is_binary(blake3) do
    ensure_consume_table!()
    :ets.delete(@consume_table, {jti, String.downcase(blake3)})
    :ok
  end

  def release_claim(_, _), do: :ok

  @doc false
  def force_prune_expired_claims! do
    ensure_consume_table!()
    prune_expired_claims!()
  end

  @doc false
  def consume_exp(jti, blake3) when is_binary(jti) and is_binary(blake3) do
    ensure_consume_table!()

    case :ets.lookup(@consume_table, {jti, String.downcase(blake3)}) do
      [{_, exp}] when is_integer(exp) -> {:ok, exp}
      _ -> :error
    end
  end

  @doc "Verify signature + claims without consume (tests / diagnostics)."
  @spec verify(String.t()) :: {:ok, map()} | {:error, term()}
  def verify(token) when is_binary(token) do
    with {:ok, %{"kid" => kid}} <- peek_kid(token),
         {:ok, signer} <- Jwks.signer_for_kid(kid) do
      Joken.verify_and_validate(verify_config(), token, signer)
    end
  end

  def verify(_), do: {:error, :invalid_token}

  defp verify_config do
    issuer = Application.fetch_env!(:world, :assets_jwt_issuer)

    Joken.Config.default_claims(iss: issuer, aud: @aud)
    |> Joken.Config.add_claim("typ", fn -> @typ end, &(&1 == @typ))
    |> Joken.Config.add_claim("sub", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("room_id", fn -> nil end, &is_binary/1)
    |> Joken.Config.add_claim("blake3s", fn -> [] end, &valid_blake3s?/1)
    |> Joken.Config.add_claim("jti", fn -> nil end, &is_binary/1)
  end

  defp match_room(%{"room_id" => ticket_room}, room_id) when ticket_room == room_id, do: :ok
  defp match_room(_, _), do: {:error, :room_mismatch}

  defp blake3_allowed(%{"blake3s" => list}, blake3) when is_list(list) do
    down = String.downcase(blake3)

    if down in Enum.map(list, &String.downcase/1) do
      :ok
    else
      {:error, :blake3_not_in_ticket}
    end
  end

  defp blake3_allowed(_, _), do: {:error, :blake3_not_in_ticket}

  defp require_participant(%{"sub" => sub}, room_id) when is_binary(sub) do
    if RoomParticipants.participant?(room_id, sub) do
      :ok
    else
      {:error, :not_participant}
    end
  end

  defp require_participant(_, _), do: {:error, :not_participant}

  # Value = ticket `exp` so prune cannot open a reuse window while JWT still verifies.
  defp claim(jti, blake3, exp)
       when is_binary(jti) and is_binary(blake3) and is_integer(exp) and exp > 0 do
    key = {jti, String.downcase(blake3)}

    case :ets.insert_new(@consume_table, {key, exp}) do
      true -> :ok
      false -> {:error, :already_consumed}
    end
  end

  defp claim(_, _, _), do: {:error, :invalid_jti}

  defp maybe_prune_expired_claims! do
    now_ms = System.monotonic_time(:millisecond)
    last = :persistent_term.get(@prune_meta_key, 0)

    if now_ms - last >= @prune_interval_ms do
      :persistent_term.put(@prune_meta_key, now_ms)
      prune_expired_claims!()
    else
      :ok
    end
  end

  defp prune_expired_claims! do
    now = System.system_time(:second)

    :ets.select_delete(@consume_table, [
      {
        {:"$1", :"$2"},
        [{:<, :"$2", now}],
        [true]
      }
    ])

    :ok
  end

  defp valid_blake3s?(list) when is_list(list) and list != [] do
    Enum.all?(list, fn b3 ->
      is_binary(b3) and byte_size(b3) == @blake3_hex_bytes and
        Regex.match?(~r/^[0-9a-fA-F]+$/, b3)
    end)
  end

  defp valid_blake3s?(_), do: false

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
end
