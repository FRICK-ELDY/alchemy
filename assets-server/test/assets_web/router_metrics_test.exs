defmodule AssetsWeb.RouterMetricsTest do
  @moduledoc """
  Public Phoenix Endpoint must not expose `/metrics` (x0049 Tunnel review).
  Metrics live on the dedicated loopback Bandit only.
  """

  use AssetsWeb.ConnCase, async: true

  test "GET /metrics on public Endpoint is 404", %{conn: conn} do
    conn = get(conn, "/metrics")
    assert conn.status == 404
  end

  test "GET /health on public Endpoint still works", %{conn: conn} do
    conn = get(conn, "/health")
    assert conn.status == 200
  end
end
