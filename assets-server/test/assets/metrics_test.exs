defmodule Assets.MetricsTest do
  use ExUnit.Case, async: false

  alias Assets.Metrics

  setup do
    Metrics.ensure_table()
    :ok
  end

  test "inc and render include fixed CAS names" do
    before = Metrics.get(:cas_get_total)
    Metrics.inc(:cas_get_total)
    Metrics.inc(:cas_quota_exceeded_total)
    Metrics.observe_ms(:cas_get_last_ms, 7)

    text = Metrics.render()
    assert text =~ "alchemy_cas_get_total{service=\"assets\"} #{before + 1}"
    assert text =~ "alchemy_cas_quota_exceeded_total{service=\"assets\"}"
    assert text =~ "alchemy_cas_get_last_ms{service=\"assets\"} 7"
  end

  test "loopback? accepts v4, v6, and IPv4-mapped" do
    assert Metrics.loopback?({127, 0, 0, 1})
    assert Metrics.loopback?({127, 1, 2, 3})
    assert Metrics.loopback?({0, 0, 0, 0, 0, 0, 0, 1})
    # ::ffff:127.0.0.1
    assert Metrics.loopback?({0, 0, 0, 0, 0, 65_535, 0x7F00, 0x0001})
    refute Metrics.loopback?({10, 0, 0, 1})
    refute Metrics.loopback?({0, 0, 0, 0, 0, 65_535, 0x0A00, 0x0001})
  end

  test "parse_bind_addr rejects non-loopback and accepts localhost alias" do
    assert {:ok, {127, 0, 0, 1}, 9103} = Metrics.parse_bind_addr("127.0.0.1:9103")
    assert {:ok, {127, 0, 0, 1}, 9103} = Metrics.parse_bind_addr("localhost:9103")
    assert {:error, msg} = Metrics.parse_bind_addr("0.0.0.0:9103")
    assert msg =~ "loopback"
    assert {:error, msg} = Metrics.parse_bind_addr("example.com:9103")
    assert msg =~ "DNS" or msg =~ "127.0.0.1"
  end
end
