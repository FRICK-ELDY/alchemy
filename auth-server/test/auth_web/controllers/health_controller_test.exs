defmodule AuthWeb.HealthControllerTest do
  use AuthWeb.ConnCase, async: false

  test "GET /health", %{conn: conn} do
    conn = get(conn, ~p"/health")

    assert %{
             "status" => "ok",
             "service" => "alchemy-auth",
             "version" => version
           } = json_response(conn, 200)

    assert is_binary(version)
  end

  test "GET /ready", %{conn: conn} do
    conn = get(conn, ~p"/ready")

    assert %{
             "status" => "ok",
             "service" => "alchemy-auth",
             "checks" => %{"repo" => "ok", "keys" => "ok"}
           } = json_response(conn, 200)
  end

  test "GET /ready returns 503 when Keys is down", %{conn: conn} do
    :ok = Supervisor.terminate_child(Auth.Supervisor, Auth.Token.Keys)

    assert %{
             "status" => "not_ready",
             "service" => "alchemy-auth",
             "checks" => %{"repo" => "ok", "keys" => "error"}
           } = json_response(get(conn, ~p"/ready"), 503)
  after
    restart_child!(Auth.Supervisor, Auth.Token.Keys)
    Auth.Token.Keys.await_ready()
  end

  test "GET /ready returns 503 when Repo is down", %{conn: conn} do
    :ok = Supervisor.terminate_child(Auth.Supervisor, Auth.Repo)

    assert %{
             "status" => "not_ready",
             "service" => "alchemy-auth",
             "checks" => %{"repo" => "error", "keys" => "ok"}
           } = json_response(get(conn, ~p"/ready"), 503)
  after
    restart_repo!()
  end

  defp restart_child!(sup, id) do
    case Supervisor.restart_child(sup, id) do
      {:ok, _} -> :ok
      {:ok, _, _} -> :ok
      {:error, {:already_started, _}} -> :ok
      {:error, :running} -> :ok
      other -> flunk("failed to restart #{inspect(id)}: #{inspect(other)}")
    end
  end

  defp restart_repo! do
    restart_child!(Auth.Supervisor, Auth.Repo)
    # Repo restart leaves Sandbox in auto; restore manual for later ConnCase owners.
    Ecto.Adapters.SQL.Sandbox.mode(Auth.Repo, :manual)
  end
end
