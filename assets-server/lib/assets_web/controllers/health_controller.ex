defmodule AssetsWeb.HealthController do
  use AssetsWeb, :controller

  alias Assets.Ready

  def index(conn, _params) do
    json(conn, %{status: "ok", service: "assets-server", slice: "C-A"})
  end

  @doc """
  Readiness: Repo + BLAKE3 Port (x008b). Liveness stays on `/health`.
  """
  def ready(conn, _params) do
    case Ready.check() do
      {:ok, checks} ->
        json(conn, %{status: "ok", service: "assets-server", checks: stringify_checks(checks)})

      {:error, checks} ->
        conn
        |> put_status(:service_unavailable)
        |> json(%{
          status: "not_ready",
          service: "assets-server",
          checks: stringify_checks(checks)
        })
    end
  end

  defp stringify_checks(checks) do
    Map.new(checks, fn {k, v} -> {k, Atom.to_string(v)} end)
  end
end
