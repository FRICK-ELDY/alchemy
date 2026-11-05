defmodule AuthWeb.RouterMetricsTest do
  @moduledoc """
  Public Phoenix Endpoint must not expose `/metrics` (x007a).
  """

  use AuthWeb.ConnCase, async: true

  test "GET /metrics on public Endpoint is 404", %{conn: conn} do
    conn = get(conn, "/metrics")
    assert conn.status == 404
  end

  test "GET /health on public Endpoint still works", %{conn: conn} do
    conn = get(conn, "/health")
    assert conn.status == 200
  end
end
