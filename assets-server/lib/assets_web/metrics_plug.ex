defmodule AssetsWeb.MetricsPlug do
  @moduledoc """
  Prometheus text on a **dedicated loopback listener** (x0049 review).

  Not mounted on `AssetsWeb.Endpoint` — Tunnel/reverse-proxy to the public port
  must never see `/metrics` (peer would look like 127.0.0.1).
  """

  @behaviour Plug

  import Plug.Conn

  alias Assets.Metrics

  @impl true
  def init(opts), do: opts

  @impl true
  def call(conn, _opts) do
    cond do
      not Metrics.enabled?() ->
        conn |> send_resp(404, "not found") |> halt()

      not Metrics.loopback?(conn.remote_ip) ->
        conn |> send_resp(404, "not found") |> halt()

      conn.method == "GET" and conn.request_path == "/metrics" ->
        body = Metrics.render()

        conn
        |> put_resp_content_type("text/plain")
        |> send_resp(200, body)
        |> halt()

      true ->
        conn |> send_resp(404, "not found") |> halt()
    end
  end
end
