defmodule World.HttpPop do
  @moduledoc """
  HTTP Proof-of-Possession for host Room Token routes (x0081 / x00d4).

  Guarantees: Ed25519 over `alchemy/pop/v1|{METHOD}|{path}|{ts}|{nonce}|{raw_body}`
  verified against JWT `cnf.spk`; clock skew reject; `{jti, nonce}` single-use until
  `pop_ts + skew` (covers future-skewed captures).
  Non-guarantees: PlaceRead / auth-JWT routes; RFC 9449 DPoP JWT shape;
  persisting nonce consume store across restart; releasing a consumed nonce on
  downstream failure (host must use a fresh nonce every request).
  """

  alias World.HttpPop.Nonce
  alias World.Token

  @ts_header "x-alchemy-pop-ts"
  @nonce_header "x-alchemy-pop-nonce"
  @sig_header "x-alchemy-pop-sig"
  @skew_secs 60
  @max_nonce_bytes 128

  @doc "Accepted `|now - pop_ts|` window (seconds). Nonce ETS `exp = pop_ts + skew_secs()`."
  @spec skew_secs() :: pos_integer()
  def skew_secs, do: @skew_secs

  @type verify_error ::
          :missing_pop
          | :invalid_pop_ts
          | :pop_ts_skew
          | :invalid_pop_nonce
          | :pop_nonce_reused
          | :invalid_pop_sig
          | :pop_verify_failed
          | :missing_cnf_spk
          | :invalid_cnf_spk
          | :missing_jti

  @doc """
  Verify PoP headers on `conn` against Room Token claims.

  `raw_body` must be the exact request body bytes appended to the signing message
  (empty for GET/DELETE with no body). `method` is upper-case HTTP verb; `path` is
  `conn.request_path`.
  """
  @spec verify(Plug.Conn.t(), map(), String.t(), String.t(), binary()) ::
          :ok | {:error, verify_error()}
  def verify(conn, claims, method, path, raw_body)
      when is_map(claims) and is_binary(method) and is_binary(path) and is_binary(raw_body) do
    with {:ok, ts} <- fetch_ts(conn),
         :ok <- check_skew(ts),
         {:ok, nonce} <- fetch_nonce(conn),
         {:ok, sig} <- fetch_sig(conn),
         {:ok, pub} <- fetch_spk(claims),
         {:ok, jti} <- fetch_jti(claims),
         msg <- signing_bytes(method, path, ts, nonce, raw_body),
         true <- :crypto.verify(:eddsa, :none, msg, sig, [pub, :ed25519]),
         :ok <- Nonce.claim(jti, nonce, ts) do
      :ok
    else
      {:error, reason} -> {:error, reason}
      false -> {:error, :pop_verify_failed}
    end
  end

  def verify(_, _, _, _, _), do: {:error, :missing_pop}

  @doc "Canonical message bytes (lockstep with `alchemy_protocol::http_pop`)."
  @spec signing_bytes(String.t(), String.t(), non_neg_integer(), String.t(), binary()) :: binary()
  def signing_bytes(method, path, ts, nonce, body)
      when is_binary(method) and is_binary(path) and is_integer(ts) and is_binary(nonce) and
             is_binary(body) do
    "alchemy/pop/v1|#{method}|#{path}|#{ts}|#{nonce}|" <> body
  end

  defp fetch_ts(conn) do
    case Plug.Conn.get_req_header(conn, @ts_header) do
      [raw] when is_binary(raw) and raw != "" ->
        case Integer.parse(raw) do
          {ts, ""} when ts >= 0 -> {:ok, ts}
          _ -> {:error, :invalid_pop_ts}
        end

      _ ->
        {:error, :missing_pop}
    end
  end

  defp fetch_nonce(conn) do
    case Plug.Conn.get_req_header(conn, @nonce_header) do
      [raw] when is_binary(raw) and raw != "" and byte_size(raw) <= @max_nonce_bytes ->
        {:ok, raw}

      [raw] when is_binary(raw) ->
        {:error, :invalid_pop_nonce}

      _ ->
        {:error, :missing_pop}
    end
  end

  defp fetch_sig(conn) do
    case Plug.Conn.get_req_header(conn, @sig_header) do
      [raw] when is_binary(raw) and raw != "" ->
        case Base.url_decode64(raw, padding: false) do
          {:ok, sig} when byte_size(sig) == 64 -> {:ok, sig}
          _ -> {:error, :invalid_pop_sig}
        end

      _ ->
        {:error, :missing_pop}
    end
  end

  defp fetch_spk(%{"cnf" => %{"spk" => spk}}) when is_binary(spk) do
    case Token.decode_session_pub(spk) do
      {:ok, pub} -> {:ok, pub}
      {:error, _} -> {:error, :invalid_cnf_spk}
    end
  end

  defp fetch_spk(_), do: {:error, :missing_cnf_spk}

  defp fetch_jti(%{"jti" => jti}) when is_binary(jti) and jti != "", do: {:ok, jti}
  defp fetch_jti(_), do: {:error, :missing_jti}

  defp check_skew(ts) do
    now = System.system_time(:second)
    dist = abs(now - ts)

    if dist <= @skew_secs do
      :ok
    else
      {:error, :pop_ts_skew}
    end
  end
end
