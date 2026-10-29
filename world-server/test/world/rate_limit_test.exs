defmodule World.RateLimitTest do
  use ExUnit.Case, async: false

  alias World.RateLimit

  setup do
    previous = Application.get_env(:world, RateLimit, [])

    Application.put_env(:world, RateLimit,
      enabled: true,
      limits: %{room_create: %{owner: %{limit: 2, period_ms: 60_000}}}
    )

    RateLimit.reset()

    on_exit(fn ->
      Application.put_env(:world, RateLimit, previous)
      RateLimit.reset()
    end)

    :ok
  end

  test "hit allows up to limit then rate_limited" do
    cfg = %{limit: 2, period_ms: 60_000}
    assert :ok = RateLimit.hit(:room_create_owner, "sub-a", cfg)
    assert :ok = RateLimit.hit(:room_create_owner, "sub-a", cfg)
    assert {:error, :rate_limited} = RateLimit.hit(:room_create_owner, "sub-a", cfg)
  end

  test "hit is per-key" do
    cfg = %{limit: 1, period_ms: 60_000}
    assert :ok = RateLimit.hit(:room_create_owner, "sub-a", cfg)
    assert :ok = RateLimit.hit(:room_create_owner, "sub-b", cfg)
    assert {:error, :rate_limited} = RateLimit.hit(:room_create_owner, "sub-a", cfg)
  end

  test "disabled skips enforcement" do
    Application.put_env(:world, RateLimit,
      enabled: false,
      limits: %{room_create: %{owner: %{limit: 1, period_ms: 60_000}}}
    )

    cfg = %{limit: 1, period_ms: 60_000}
    assert :ok = RateLimit.hit(:room_create_owner, "sub-a", cfg)
    assert :ok = RateLimit.hit(:room_create_owner, "sub-a", cfg)
  end

  test "retry_after_seconds is at least 1 and within period" do
    seconds = RateLimit.retry_after_seconds(60_000)
    assert seconds in 1..60
  end

  test "bucket_specs cover configured room_create path" do
    assert {[:room_create, :owner], :room_create_owner} in RateLimit.bucket_specs()
  end
end
