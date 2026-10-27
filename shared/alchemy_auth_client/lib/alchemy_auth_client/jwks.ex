defmodule AlchemyAuthClient.Jwks do
  @moduledoc """
  Caches auth-server JWKS for user JWT verification.

  Start with `otp_app:` and usually `name:` (e.g. `World.Auth.Jwks`).
  Reads `:auth_jwks_url` and `:auth_jwks_fetch_on_start` from that OTP app.
  """

  use GenServer

  require Logger

  @type server :: GenServer.server()

  # Min interval between outbound JWKS fetches when cache miss storms (x001f / x002f).
  @fetch_cooldown_ms 1_000

  @doc """
  Starts the JWKS cache.

  ## Options
  - `:otp_app` (required) — application holding `:auth_jwks_url`
  - `:name` — GenServer name (default `__MODULE__`)
  - `:fetch_cooldown_ms` — unknown-kid storm guard (default #{@fetch_cooldown_ms})
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
      last_fetch_ms: 0
    }

    {:ok, state, {:continue, :initial_fetch}}
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
          # Do not advance last_fetch_ms on failure so the next call can retry immediately.
          {:noreply, state}
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

      # Empty cache (e.g. auth was down at boot) or unknown kid: try a fetch.
      :error ->
        refresh_and_lookup_kid(kid, state)
    end
  end

  def handle_call(:refresh, _from, state) do
    case do_fetch(state.otp_app) do
      {:ok, jwks, signers} ->
        now = state.now_ms.()

        {:reply, :ok,
         %{
           state
           | jwks: jwks,
             signers_by_kid: signers,
             fetched_at: div(now, 1000),
             last_fetch_ms: now
         }}

      {:error, reason} ->
        {:reply, {:error, reason}, state}
    end
  end

  def handle_call({:install_test, jwks, signers}, _from, state) do
    now = state.now_ms.()

    {:reply, :ok,
     %{
       state
       | jwks: jwks,
         signers_by_kid: signers,
         fetched_at: div(now, 1000),
         last_fetch_ms: now
     }}
  end

  defp refresh_and_lookup_kid(kid, state) do
    now = state.now_ms.()
    # Cooldown only after a successful fetch (storm guard). Failures stay retryable.
    has_keys = state.signers_by_kid != %{}

    if has_keys and now - state.last_fetch_ms < state.fetch_cooldown_ms do
      {:reply, lookup_kid(state.signers_by_kid, kid), state}
    else
      case do_fetch(state.otp_app) do
        {:ok, jwks, signers} ->
          state = %{
            state
            | jwks: jwks,
              signers_by_kid: signers,
              fetched_at: div(now, 1000),
              last_fetch_ms: now
          }

          {:reply, lookup_kid(signers, kid), state}

        {:error, _} when state.signers_by_kid == %{} ->
          {:reply, {:error, :empty_jwks}, state}

        {:error, _} ->
          {:reply, {:error, :unknown_kid}, state}
      end
    end
  end

  defp lookup_kid(signers, kid) do
    case Map.fetch(signers, kid) do
      {:ok, signer} -> {:ok, signer}
      :error -> {:error, :unknown_kid}
    end
  end

  defp do_fetch(otp_app) do
    url = Application.fetch_env!(otp_app, :auth_jwks_url)

    # Never block the GenServer on Req's default retry backoff (breaks test setup).
    case Req.get(url, receive_timeout: 2_000, retry: false) do
      {:ok, %{status: 200, body: body}} when is_map(body) ->
        {:ok, body, build_signers(body)}

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
