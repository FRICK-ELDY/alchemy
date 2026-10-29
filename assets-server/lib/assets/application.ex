defmodule Assets.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    if Application.fetch_env!(:assets, :cas_backend) == :local do
      File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    end

    Assets.Metrics.ensure_table()

    children = [
      AssetsWeb.Telemetry,
      Assets.Repo,
      {Phoenix.PubSub, name: Assets.PubSub},
      Assets.Auth.Jwks,
      # x0049: dedicated loopback Bandit — never on Endpoint (Tunnel peer spoof)
      Assets.Metrics.Server,
      AssetsWeb.Endpoint
    ]

    opts = [strategy: :one_for_one, name: Assets.Supervisor]
    Supervisor.start_link(children, opts)
  end

  @impl true
  def config_change(changed, _new, removed) do
    AssetsWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
