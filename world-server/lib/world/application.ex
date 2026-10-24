defmodule World.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    children = [
      WorldWeb.Telemetry,
      {Phoenix.PubSub, name: World.PubSub},
      World.Token.Keys,
      World.Rooms,
      WorldWeb.Endpoint
    ]

    opts = [strategy: :one_for_one, name: World.Supervisor]
    Supervisor.start_link(children, opts)
  end

  @impl true
  def config_change(changed, _new, removed) do
    WorldWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
