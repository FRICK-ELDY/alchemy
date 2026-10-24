defmodule World.Auth.Jwks do
  @moduledoc """
  Caches auth-server JWKS for user JWT verification.

  Guarantees: RS256 public keys from AUTH_JWKS_URL (or test install).
  Non-guarantees: revocation lists, user status DB lookups.
  """

  use GenServer

  require Logger

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec jwks() :: map()
  def jwks, do: GenServer.call(__MODULE__, :jwks)

  @spec signer_for_kid(String.t()) ::
          {:ok, Joken.Signer.t()} | {:error, :unknown_kid | :empty_jwks}
  def signer_for_kid(kid) when is_binary(kid),
    do: GenServer.call(__MODULE__, {:signer_for_kid, kid})

  @spec refresh() :: :ok | {:error, term()}
  def refresh, do: GenServer.call(__MODULE__, :refresh)

  @doc false
  def install_test_jwks!(jwks, signers_by_kid) when is_map(jwks) and is_map(signers_by_kid) do
    GenServer.call(__MODULE__, {:install_test, jwks, signers_by_kid})
  end

  @impl true
  def init(_opts) do
    {:ok, %{jwks: %{"keys" => []}, signers_by_kid: %{}, fetched_at: nil},
     {:continue, :initial_fetch}}
  end

  @impl true
  def handle_continue(:initial_fetch, state) do
    case do_fetch() do
      {:ok, jwks, signers} ->
        {:noreply,
         %{
           state
           | jwks: jwks,
             signers_by_kid: signers,
             fetched_at: System.monotonic_time(:second)
         }}

      {:error, reason} ->
        Logger.warning("auth JWKS initial fetch failed: #{inspect(reason)}")
        {:noreply, state}
    end
  end

  @impl true
  def handle_call(:jwks, _from, state), do: {:reply, state.jwks, state}

  @impl true
  def handle_call({:signer_for_kid, kid}, _from, state) do
    case Map.fetch(state.signers_by_kid, kid) do
      {:ok, signer} ->
        {:reply, {:ok, signer}, state}

      :error when state.signers_by_kid == %{} ->
        {:reply, {:error, :empty_jwks}, state}

      :error ->
        refresh_and_lookup_kid(kid, state)
    end
  end

  @impl true
  def handle_call(:refresh, _from, state) do
    case do_fetch() do
      {:ok, jwks, signers} ->
        {:reply, :ok,
         %{
           state
           | jwks: jwks,
             signers_by_kid: signers,
             fetched_at: System.monotonic_time(:second)
         }}

      {:error, reason} ->
        {:reply, {:error, reason}, state}
    end
  end

  @impl true
  def handle_call({:install_test, jwks, signers}, _from, state) do
    {:reply, :ok,
     %{state | jwks: jwks, signers_by_kid: signers, fetched_at: System.monotonic_time(:second)}}
  end

  defp refresh_and_lookup_kid(kid, state) do
    case do_fetch() do
      {:ok, jwks, signers} ->
        state = %{
          state
          | jwks: jwks,
            signers_by_kid: signers,
            fetched_at: System.monotonic_time(:second)
        }

        {:reply, lookup_kid(signers, kid), state}

      {:error, _} ->
        {:reply, {:error, :unknown_kid}, state}
    end
  end

  defp lookup_kid(signers, kid) do
    case Map.fetch(signers, kid) do
      {:ok, signer} -> {:ok, signer}
      :error -> {:error, :unknown_kid}
    end
  end

  defp do_fetch do
    url = Application.fetch_env!(:world, :auth_jwks_url)

    case Req.get(url, receive_timeout: 5_000) do
      {:ok, %{status: 200, body: body}} when is_map(body) ->
        signers = build_signers(body)
        {:ok, body, signers}

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
