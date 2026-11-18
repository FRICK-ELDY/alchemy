defmodule Auth.Application do
  # See https://elixir.hexdocs.pm/Application.html
  # for more information on OTP Applications
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    maybe_configure_mailer()

    Auth.Metrics.ensure_table()

    children =
      [
        AuthWeb.Telemetry,
        Auth.RateLimit,
        Auth.Repo,
        Auth.TokenCleanup,
        Auth.Token.Keys,
        {Task.Supervisor, name: Auth.TaskSupervisor},
        {DNSCluster, query: Application.get_env(:auth, :dns_cluster_query) || :ignore},
        {Phoenix.PubSub, name: Auth.PubSub},
        # x007a: dedicated loopback Bandit — never on Endpoint
        Auth.Metrics.Server
      ] ++
        relation_outbox_dispatcher() ++
        [
          # Start to serve requests, typically the last entry
          AuthWeb.Endpoint
        ]

    # See https://elixir.hexdocs.pm/Supervisor.html
    # for other strategies and supported options
    opts = [strategy: :one_for_one, name: Auth.Supervisor]
    Supervisor.start_link(children, opts)
  end

  # x00d7: HTTP outbox drain before Endpoint so kick/0 is live when Social commits.
  # Disabled in test; stub delivers sync after commit.
  defp relation_outbox_dispatcher do
    opts = Application.get_env(:auth, Auth.WorldNotify.Dispatcher, [])

    if Keyword.get(opts, :enabled, true) do
      [{Auth.WorldNotify.Dispatcher, opts}]
    else
      []
    end
  end

  defp maybe_configure_mailer do
    if Application.get_env(:auth, :configure_mailer_at_startup, false) do
      Auth.MailConfig.configure!()
    end

    :ok
  end

  # Tell Phoenix to update the endpoint configuration
  # whenever the application is updated.
  @impl true
  def config_change(changed, _new, removed) do
    AuthWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
