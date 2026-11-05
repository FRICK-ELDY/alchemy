defmodule World.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    World.Metrics.ensure_table()

    children =
      [
        WorldWeb.Telemetry,
        World.RateLimit,
        World.Repo,
        {Phoenix.PubSub, name: World.PubSub},
        World.Token.Keys,
        World.Auth.Jwks,
        World.PlaceHashes,
        # x0075: permanent manifest-seal-hash Port (replaces per-request CLI)
        World.ManifestSealHash.Worker,
        # x007a: dedicated loopback Bandit — never on Endpoint
        World.Metrics.Server,
        WorldWeb.Endpoint
      ] ++ zenoh_children()

    opts = [strategy: :one_for_one, name: World.Supervisor]
    Supervisor.start_link(children, opts)
  end

  defp zenoh_children do
    case Application.get_env(:world, :zenoh_adapter, World.Network.Zenoh.ZenohexAdapter) do
      World.Network.Zenoh.ZenohexAdapter ->
        [World.Network.Zenoh.ZenohexAdapter]

      World.Network.Zenoh.LocalStubAdapter ->
        [World.Network.Zenoh.LocalStubAdapter]

      other ->
        raise ArgumentError,
              "unsupported :world :zenoh_adapter #{inspect(other)}; expected ZenohexAdapter or LocalStubAdapter"
    end
  end

  @impl true
  def config_change(changed, _new, removed) do
    WorldWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
