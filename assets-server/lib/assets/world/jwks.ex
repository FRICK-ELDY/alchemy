defmodule Assets.World.Jwks do
  @moduledoc """
  Caches world-server JWKS for PlaceRead verification (x006b).

  Guarantees: RS256 public keys from `:world_jwks_url` (or test install).
  Non-guarantees: auth-server JWKS (see `Assets.Auth.Jwks`).
  """

  use GenServer

  require Logger

  @server __MODULE__
  @fetch_cooldown_ms 1_000

  def child_spec(opts) do
    %{
      id: __MODULE__,
      start: {__MODULE__, :start_link, [opts]},
      type: :worker,
      restart: :permanent
    }
  end

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: @server)
  end

  def jwks, do: GenServer.call(@server, :jwks)

  def signer_for_kid(kid) when is_binary(kid),
    do: GenServer.call(@server, {:signer_for_kid, kid})

  def refresh, do: GenServer.call(@server, :refresh)

  @doc false
  def install_test_jwks!(jwks, signers_by_kid)
      when is_map(jwks) and is_map(signers_by_kid) do
    GenServer.call(@server, {:install_test, jwks, signers_by_kid})
  end

  @impl true
  def init(_opts) do
    state = %{
      jwks: %{"keys" => []},
      signers_by_kid: %{},
      last_fetch_ms: 0
    }

    {:ok, state, {:continue, :initial_fetch}}
  end

  @impl true
  def handle_continue(:initial_fetch, state) do
    if Application.get_env(:assets, :world_jwks_fetch_on_start, false) and
         is_binary(Application.get_env(:assets, :world_jwks_url)) do
      case do_fetch() do
        {:ok, jwks, signers} ->
          {:noreply, %{state | jwks: jwks, signers_by_kid: signers, last_fetch_ms: now_ms()}}

        {:error, reason} ->
          Logger.warning("world JWKS initial fetch failed: #{inspect(reason)}")
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

      :error ->
        refresh_and_lookup(kid, state)
    end
  end

  def handle_call(:refresh, _from, state) do
    case do_fetch() do
      {:ok, jwks, signers} ->
        {:reply, :ok, %{state | jwks: jwks, signers_by_kid: signers, last_fetch_ms: now_ms()}}

      {:error, reason} ->
        {:reply, {:error, reason}, state}
    end
  end

  def handle_call({:install_test, jwks, signers}, _from, state) do
    {:reply, :ok, %{state | jwks: jwks, signers_by_kid: signers, last_fetch_ms: now_ms()}}
  end

  defp refresh_and_lookup(kid, state) do
    now = now_ms()
    has_keys = state.signers_by_kid != %{}

    if has_keys and now - state.last_fetch_ms < @fetch_cooldown_ms do
      {:reply, lookup(state.signers_by_kid, kid), state}
    else
      case do_fetch() do
        {:ok, jwks, signers} ->
          state = %{state | jwks: jwks, signers_by_kid: signers, last_fetch_ms: now}
          {:reply, lookup(signers, kid), state}

        {:error, _} when state.signers_by_kid == %{} ->
          {:reply, {:error, :empty_jwks}, state}

        {:error, _} ->
          {:reply, {:error, :unknown_kid}, state}
      end
    end
  end

  defp lookup(signers, kid) do
    case Map.fetch(signers, kid) do
      {:ok, signer} -> {:ok, signer}
      :error -> {:error, :unknown_kid}
    end
  end

  defp do_fetch do
    case Application.get_env(:assets, :world_jwks_url) do
      url when is_binary(url) and url != "" ->
        case Req.get(url, receive_timeout: 2_000, retry: false) do
          {:ok, %{status: 200, body: body}} when is_map(body) ->
            {:ok, body, build_signers(body)}

          {:ok, %{status: status}} ->
            {:error, {:http_status, status}}

          {:error, reason} ->
            {:error, reason}
        end

      _ ->
        {:error, :world_jwks_unconfigured}
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

  defp now_ms, do: System.monotonic_time(:millisecond)
end
