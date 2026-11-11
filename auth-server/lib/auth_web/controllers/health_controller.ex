defmodule AuthWeb.HealthController do
  use AuthWeb, :controller

  alias Auth.Ready

  @service "alchemy-auth"

  def index(conn, _params) do
    json(conn, %{
      status: "ok",
      service: @service,
      version: app_version()
    })
  end

  @doc """
  Readiness: Repo + Token.Keys (x008b). Liveness stays on `/health`.
  """
  def ready(conn, _params) do
    case Ready.check() do
      {:ok, checks} ->
        json(conn, %{
          status: "ok",
          service: @service,
          checks: stringify_checks(checks)
        })

      {:error, checks} ->
        conn
        |> put_status(:service_unavailable)
        |> json(%{
          status: "not_ready",
          service: @service,
          checks: stringify_checks(checks)
        })
    end
  end

  defp stringify_checks(checks) do
    Map.new(checks, fn {k, v} -> {k, Atom.to_string(v)} end)
  end

  defp app_version do
    case Application.spec(:auth, :vsn) do
      nil -> "unknown"
      vsn -> to_string(vsn)
    end
  end
end
