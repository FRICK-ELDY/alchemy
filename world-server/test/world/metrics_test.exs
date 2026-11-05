defmodule World.MetricsTest do
  use ExUnit.Case, async: false

  alias World.Metrics

  setup do
    Metrics.ensure_table()
    :ok
  end

  test "inc and render include fixed token-issue names" do
    before = Metrics.get(:token_issue_total)
    Metrics.inc(:token_issue_total)
    Metrics.inc(:token_issue_errors_total)

    text = Metrics.render()
    assert text =~ "alchemy_world_token_issue_total #{before + 1}"
    assert text =~ "alchemy_world_token_issue_errors_total"
  end

  test "parse_bind_addr rejects non-loopback and accepts localhost alias" do
    assert {:ok, {127, 0, 0, 1}, 9105} = Metrics.parse_bind_addr("127.0.0.1:9105")
    assert {:ok, {127, 0, 0, 1}, 9105} = Metrics.parse_bind_addr("localhost:9105")
    assert {:error, msg} = Metrics.parse_bind_addr("0.0.0.0:9105")
    assert msg =~ "loopback"
  end
end
