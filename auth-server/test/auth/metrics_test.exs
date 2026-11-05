defmodule Auth.MetricsTest do
  use ExUnit.Case, async: false

  alias Auth.Metrics

  setup do
    Metrics.ensure_table()
    :ok
  end

  test "inc and render include fixed login names" do
    before = Metrics.get(:login_total)
    Metrics.inc(:login_total)
    Metrics.inc(:login_errors_total)

    text = Metrics.render()
    assert text =~ "alchemy_auth_login_total #{before + 1}"
    assert text =~ "alchemy_auth_login_errors_total"
  end

  test "parse_bind_addr rejects non-loopback" do
    assert {:ok, {127, 0, 0, 1}, 9104} = Metrics.parse_bind_addr("127.0.0.1:9104")
    assert {:error, msg} = Metrics.parse_bind_addr("0.0.0.0:9104")
    assert msg =~ "loopback"
  end
end
