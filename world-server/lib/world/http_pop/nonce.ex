defmodule World.HttpPop.Nonce do
  @moduledoc """
  Single-use `{jti, nonce}` store for HTTP PoP (x00d4).

  Guarantees: `insert_new` rejects reuse until `exp = pop_ts + skew` (covers the full
  remaining skew window for future-skewed timestamps); expired rows are pruned.
  Non-guarantees: persistence across world restart (short replay window after boot);
  releasing a claim on downstream failure (host must send a fresh nonce each request).
  """

  alias World.HttpPop

  @consume_table __MODULE__.Consumed
  @prune_interval_ms 5_000
  @prune_meta_key {__MODULE__, :last_prune_ms}
  @max_nonce_bytes 128

  @doc false
  def ensure_table! do
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
  Claim `{jti, nonce}` once.

  `pop_ts` is the verified `x-alchemy-pop-ts`. Stored `exp` is
  `pop_ts + HttpPop.skew_secs()` so a future-skewed capture cannot be replayed after a
  claim-relative skew prune while `|now - pop_ts| <= skew` still holds.
  """
  @spec claim(String.t(), String.t(), non_neg_integer()) ::
          :ok | {:error, :pop_nonce_reused | :invalid_pop_nonce}
  def claim(jti, nonce, pop_ts)
      when is_binary(jti) and jti != "" and is_binary(nonce) and nonce != "" and
             byte_size(nonce) <= @max_nonce_bytes and is_integer(pop_ts) and pop_ts >= 0 do
    ensure_table!()
    maybe_prune_expired!()
    exp = pop_ts + HttpPop.skew_secs()

    case :ets.insert_new(@consume_table, {{jti, nonce}, exp}) do
      true -> :ok
      false -> {:error, :pop_nonce_reused}
    end
  end

  def claim(_, _, _), do: {:error, :invalid_pop_nonce}

  @doc false
  def consume_exp(jti, nonce) when is_binary(jti) and is_binary(nonce) do
    ensure_table!()

    case :ets.lookup(@consume_table, {jti, nonce}) do
      [{_, exp}] when is_integer(exp) -> {:ok, exp}
      _ -> :error
    end
  end

  def consume_exp(_, _), do: :error

  @doc false
  def force_prune_expired! do
    ensure_table!()
    prune_expired!()
  end

  @doc false
  def put_exp_for_test!(jti, nonce, exp)
      when is_binary(jti) and is_binary(nonce) and is_integer(exp) do
    ensure_table!()
    true = :ets.insert(@consume_table, {{jti, nonce}, exp})
    :ok
  end

  defp maybe_prune_expired! do
    now_ms = System.monotonic_time(:millisecond)
    last = :persistent_term.get(@prune_meta_key, 0)

    if now_ms - last >= @prune_interval_ms do
      :persistent_term.put(@prune_meta_key, now_ms)
      prune_expired!()
    else
      :ok
    end
  end

  defp prune_expired! do
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
end
