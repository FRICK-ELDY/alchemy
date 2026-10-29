defmodule AssetsWeb.MetricsPlugTest do
  use ExUnit.Case, async: false
  import Plug.Test

  alias Assets.Metrics
  alias AssetsWeb.MetricsPlug

  setup do
    Metrics.ensure_table()
    previous_flag = System.get_env("ASSETS_METRICS")
    previous_addr = System.get_env("ASSETS_METRICS_ADDR")

    on_exit(fn ->
      restore_env("ASSETS_METRICS", previous_flag)
      restore_env("ASSETS_METRICS_ADDR", previous_addr)
    end)

    :ok
  end

  defp restore_env(name, nil), do: System.delete_env(name)
  defp restore_env(name, value), do: System.put_env(name, value)

  defp call(conn) do
    MetricsPlug.call(conn, MetricsPlug.init([]))
  end

  test "disabled → 404 even from loopback" do
    System.delete_env("ASSETS_METRICS")
    System.delete_env("ASSETS_METRICS_ADDR")

    conn =
      conn(:get, "/metrics")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> call()

    assert conn.status == 404
  end

  test "enabled + loopback GET /metrics → 200 with CAS names" do
    System.put_env("ASSETS_METRICS", "1")
    System.delete_env("ASSETS_METRICS_ADDR")

    conn =
      conn(:get, "/metrics")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> call()

    assert conn.status == 200
    assert conn.resp_body =~ "alchemy_cas_get_total"
  end

  test "enabled + non-loopback → 404" do
    System.put_env("ASSETS_METRICS", "1")

    conn =
      conn(:get, "/metrics")
      |> Map.put(:remote_ip, {10, 0, 0, 1})
      |> call()

    assert conn.status == 404
  end

  test "enabled + loopback but wrong path → 404" do
    System.put_env("ASSETS_METRICS", "1")

    conn =
      conn(:get, "/health")
      |> Map.put(:remote_ip, {127, 0, 0, 1})
      |> call()

    assert conn.status == 404
  end
end
