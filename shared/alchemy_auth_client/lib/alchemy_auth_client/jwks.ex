defmodule AlchemyAuthClient.Jwks do
  @moduledoc """
  Caches auth-server JWKS for user JWT verification.

  Start with `otp_app:` and usually `name:` (e.g. `World.Auth.Jwks`).
  Reads `:auth_jwks_url` and `:auth_jwks_fetch_on_start` from that OTP app.

  Guarantees (x008c): fetch **failures** stamp the same cooldown as successes;
  `signer_for_kid` miss path replies from cache only (no GenServer-blocking HTTP);
  in-flight refreshes coalesce. Non-guarantees: host-style soft/hard TTL schedule;
  auth revocation lists; same HTTP request succeeding after a mid-request key rotate
  (caller may retry on `:unknown_kid` / `:empty_jwks`).
  """

  use GenServer

  require Logger

  @type server :: GenServer.server()

  # Min interval between outbound JWKS fetches (success or failure; x001f / x008c).
  @fetch_cooldown_ms 1_000

  @doc """
  Starts the JWKS cache.

  ## Options
  - `:otp_app` (required) — application holding `:auth_jwks_url`
  - `:name` — GenServer name (default `__MODULE__`)
  - `:fetch_cooldown_ms` — storm / failure guard (default #{@fetch_cooldown_ms})
  - `:now_ms` — `(() -> integer)` monotonic clock for tests (default `System.monotonic_time/1`)
  """
  def start_link(opts) do
    otp_app = Keyword.fetch!(opts, :otp_app)
    name = Keyword.get(opts, :name, __MODULE__)
    cooldown = Keyword.get(opts, :fetch_cooldown_ms, @fetch_cooldown_ms)
    now_ms = Keyword.get(opts, :now_ms, fn -> System.monotonic_time(:millisecond) end)

    GenServer.start_link(
      __MODULE__,
      %{otp_app: otp_app, fetch_cooldown_ms: cooldown, now_ms: now_ms},
      name: name
    )
  end

  @spec jwks(server()) :: map()
  def jwks(server \\ __MODULE__), do: GenServer.call(server, :jwks)

  @spec signer_for_kid(server(), String.t()) ::
          {:ok, Joken.Signer.t()} | {:error, :unknown_kid | :empty_jwks}
  def signer_for_kid(server \\ __MODULE__, kid) when is_binary(kid),
    do: GenServer.call(server, {:signer_for_kid, kid})

  @spec refresh(server()) :: :ok | {:error, term()}
  def refresh(server \\ __MODULE__), do: GenServer.call(server, :refresh)

  @doc false
  def install_test_jwks!(server \\ __MODULE__, jwks, signers_by_kid)
      when is_map(jwks) and is_map(signers_by_kid) do
    GenServer.call(server, {:install_test, jwks, signers_by_kid})
  end

  # Test helper: wait until no **async** refresh job is in flight (not "fully idle";
  # sync refresh may still run). Abandon-during-sync does not wake waiters early.
  @doc false
  @spec await_idle(server(), pos_integer()) :: :ok
  def await_idle(server \\ __MODULE__, timeout_ms \\ 5_000)
      when is_integer(timeout_ms) and timeout_ms > 0 do
    GenServer.call(server, :await_idle, timeout_ms + 1_000)
  end

  @spec fetch_cooldown_ms() :: pos_integer()
  def fetch_cooldown_ms, do: @fetch_cooldown_ms

  @impl true
  def init(%{otp_app: otp_app, fetch_cooldown_ms: cooldown, now_ms: now_ms}) do
    state = %{
      otp_app: otp_app,
      fetch_cooldown_ms: cooldown,
      now_ms: now_ms,
      jwks: %{"keys" => []},
      signers_by_kid: %{},
      fetched_at: nil,
      # nil = never attempted (do not use 0; injected clocks often start at 0).
      last_fetch_ms: nil,
      refresh_job: nil,
      idle_waiters: []
    }

    {:ok, state, {:continue, :initial_fetch}}
  end

  @impl true
  def terminate(_reason, state) do
    _ = abandon_refresh_job(state)
    :ok
  end

  @impl true
  def handle_continue(:initial_fetch, state) do
    if Application.get_env(state.otp_app, :auth_jwks_fetch_on_start, true) do
      case do_fetch(state.otp_app) do
        {:ok, jwks, signers} ->
          now = state.now_ms.()

          {:noreply,
           %{
             state
             | jwks: jwks,
               signers_by_kid: signers,
               fetched_at: div(now, 1000),
               last_fetch_ms: now
           }}

        {:error, reason} ->
          Logger.warning("auth JWKS initial fetch failed: #{inspect(reason)}")
          # Stamp failure so miss storms do not immediately re-HTTP (x008c).
          {:noreply, %{state | last_fetch_ms: state.now_ms.()}}
      end
    else
      {:noreply, state}
    end
  end

  @impl true
  def handle_call(:jwks, _from, state), do: {:reply, state.jwks, state}

  def handle_call({:signer_for_kid, kid}, _from, state) do
    case Map.fetch(state.signers_by_kid, kid) do
      {:ok, signer} ->
        {:reply, {:ok, signer}, state}

      :error ->
        state = maybe_kick_refresh(state)
        {:reply, lookup_kid(state.signers_by_kid, kid), state}
    end
  end

  def handle_call(:refresh, _from, state) do
    # Explicit refresh stays synchronous (tests / ops). Drop in-flight job bookkeeping
    # so a late async result cannot overwrite a newer sync outcome.
    # Do not wake await_idle waiters here — sync HTTP is still about to run.
    state = abandon_refresh_job(state)

    case do_fetch(state.otp_app) do
      {:ok, jwks, signers} ->
        now = state.now_ms.()

        state = %{
          state
          | jwks: jwks,
            signers_by_kid: signers,
            fetched_at: div(now, 1000),
            last_fetch_ms: now
        }

        {:reply, :ok, reply_idle_waiters(state)}

      {:error, reason} ->
        now = state.now_ms.()
        state = reply_idle_waiters(%{state | last_fetch_ms: now})
        {:reply, {:error, reason}, state}
    end
  end

  def handle_call({:install_test, jwks, signers}, _from, state) do
    now = state.now_ms.()
    state = abandon_refresh_job(state)

    state = %{
      state
      | jwks: jwks,
        signers_by_kid: signers,
        fetched_at: div(now, 1000),
        last_fetch_ms: now
    }

    {:reply, :ok, reply_idle_waiters(state)}
  end

  def handle_call(:await_idle, from, state) do
    # Async-job idle only (see moduledoc on await_idle/2).
    if state.refresh_job == nil do
      {:reply, :ok, state}
    else
      {:noreply, %{state | idle_waiters: [from | state.idle_waiters]}}
    end
  end

  @impl true
  def handle_info({:jwks_fetch_result, ref, result}, %{refresh_job: %{ref: ref}} = state) do
    Process.demonitor(state.refresh_job.mon, [:flush])
    state = apply_fetch_result(result, %{state | refresh_job: nil})
    {:noreply, reply_idle_waiters(state)}
  end

  def handle_info({:jwks_fetch_result, _ref, _result}, state) do
    # Late result after sync refresh / abandon.
    {:noreply, state}
  end

  # send(result) then exit(:normal): same-process order puts result before DOWN.
  # Result handler demonitor(:flush) drops the DOWN. Do **not** clear refresh_job or
  # wake waiters here — that would idle without applying the fetch.
  def handle_info({:DOWN, mon, :process, _pid, reason}, %{refresh_job: %{mon: mon}} = state)
      when reason in [:normal, :shutdown] do
    {:noreply, state}
  end

  def handle_info({:DOWN, mon, :process, _pid, reason}, %{refresh_job: %{mon: mon}} = state) do
    Logger.warning("auth JWKS async fetch crashed: #{inspect(reason)}")
    now = state.now_ms.()

    state =
      reply_idle_waiters(%{
        state
        | refresh_job: nil,
          last_fetch_ms: now
      })

    {:noreply, state}
  end

  def handle_info({:DOWN, _mon, :process, _pid, _reason}, state), do: {:noreply, state}

  defp maybe_kick_refresh(state) do
    now = state.now_ms.()

    cond do
      state.refresh_job != nil ->
        state

      cooled?(state, now) ->
        state

      true ->
        job = start_fetch_job(state.otp_app)
        %{state | refresh_job: job}
    end
  end

  defp cooled?(%{last_fetch_ms: nil}, _now), do: false

  defp cooled?(state, now) do
    now - state.last_fetch_ms < state.fetch_cooldown_ms
  end

  defp start_fetch_job(otp_app) do
    parent = self()
    ref = make_ref()

    {pid, mon} =
      spawn_monitor(fn -> send(parent, {:jwks_fetch_result, ref, do_fetch(otp_app)}) end)

    %{ref: ref, pid: pid, mon: mon}
  end

  defp abandon_refresh_job(%{refresh_job: nil} = state), do: state

  defp abandon_refresh_job(%{refresh_job: job} = state) do
    Process.demonitor(job.mon, [:flush])
    # Best-effort; result may still arrive and be ignored by ref mismatch.
    Process.exit(job.pid, :kill)
    # Keep idle_waiters: sync refresh/install will reply after it finishes.
    %{state | refresh_job: nil}
  end

  defp apply_fetch_result({:ok, jwks, signers}, state) do
    now = state.now_ms.()

    %{
      state
      | jwks: jwks,
        signers_by_kid: signers,
        fetched_at: div(now, 1000),
        last_fetch_ms: now
    }
  end

  defp apply_fetch_result({:error, reason}, state) do
    Logger.warning("auth JWKS fetch failed: #{inspect(reason)}")
    %{state | last_fetch_ms: state.now_ms.()}
  end

  defp reply_idle_waiters(%{idle_waiters: []} = state), do: state

  defp reply_idle_waiters(%{idle_waiters: waiters} = state) do
    Enum.each(waiters, &GenServer.reply(&1, :ok))
    %{state | idle_waiters: []}
  end

  defp lookup_kid(signers, kid) do
    case Map.fetch(signers, kid) do
      {:ok, signer} ->
        {:ok, signer}

      :error when signers == %{} ->
        {:error, :empty_jwks}

      :error ->
        {:error, :unknown_kid}
    end
  end

  defp do_fetch(otp_app) do
    url = Application.fetch_env!(otp_app, :auth_jwks_url)

    # Never block with Req's default retry backoff (breaks test setup).
    # Decode JSON ourselves so non-200 bodies (e.g. "down") are not Jason errors.
    case Req.get(url, receive_timeout: 2_000, retry: false, decode_body: false) do
      {:ok, %{status: 200, body: body}} when is_binary(body) ->
        case Jason.decode(body) do
          {:ok, map} when is_map(map) ->
            {:ok, map, build_signers(map)}

          {:ok, _} ->
            {:error, :invalid_jwks}

          {:error, reason} ->
            {:error, reason}
        end

      {:ok, %{status: status}} ->
        {:error, {:http_status, status}}

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp build_signers(%{"keys" => keys}) when is_list(keys) do
    Enum.reduce(keys, %{}, fn key, acc ->
      with "RSA" <- Map.get(key, "kty"),
           kid when is_binary(kid) and kid != "" <- Map.get(key, "kid"),
           n when is_binary(n) <- Map.get(key, "n"),
           e when is_binary(e) <- Map.get(key, "e") do
        jwk = %{"kty" => "RSA", "n" => n, "e" => e, "kid" => kid}
        Map.put(acc, kid, Joken.Signer.create("RS256", jwk))
      else
        _ -> acc
      end
    end)
  end

  defp build_signers(_), do: %{}
end
