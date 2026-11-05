defmodule AlchemyRateLimitTest do
  use ExUnit.Case, async: false

  @server AlchemyRateLimit.TestServer
  @table :alchemy_rate_limit_test
  @config_key AlchemyRateLimit.TestConfig

  setup do
    Application.put_env(:alchemy_rate_limit, @config_key,
      enabled: true,
      limits: %{
        login: %{ip: %{limit: 2, period_ms: 60_000}},
        register: %{email: %{limit: 2, period_ms: 3_600_000}}
      }
    )

    pid =
      start_supervised!(
        {AlchemyRateLimit,
         name: @server,
         table: @table,
         otp_app: :alchemy_rate_limit,
         config_key: @config_key,
         telemetry_prefix: [:alchemy_rate_limit, :rate_limit],
         log_event: "alchemy_rate_limit.test.throttle",
         buckets: [
           login_ip: {:login, :ip},
           register_email: {:register, :email}
         ]}
      )

    on_exit(fn ->
      Application.delete_env(:alchemy_rate_limit, @config_key)
    end)

    %{pid: pid}
  end

  test "allows requests up to the configured limit" do
    config = %{limit: 2, period_ms: 60_000}

    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
    assert {:error, :rate_limited} = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
  end

  test "tracks buckets and keys independently" do
    config = %{limit: 1, period_ms: 60_000}

    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "10.0.0.1", config)
    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "10.0.0.2", config)
    assert :ok = AlchemyRateLimit.hit(@server, :register_email, "a@example.com", config)
    assert {:error, :rate_limited} = AlchemyRateLimit.hit(@server, :login_ip, "10.0.0.1", config)
  end

  test "disabled skips counting" do
    Application.put_env(:alchemy_rate_limit, @config_key, enabled: false)
    config = %{limit: 1, period_ms: 60_000}

    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
  end

  test "reset clears counters" do
    config = %{limit: 1, period_ms: 60_000}

    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
    assert {:error, :rate_limited} = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)

    assert :ok = AlchemyRateLimit.reset(@server)
    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
  end

  test "emits telemetry when throttled" do
    handler_id = "alchemy-rate-limit-test-#{System.unique_integer()}"

    :ok =
      :telemetry.attach(
        handler_id,
        [:alchemy_rate_limit, :rate_limit, :throttle],
        fn event, measurements, metadata, _config ->
          send(self(), {event, measurements, metadata})
        end,
        nil
      )

    on_exit(fn -> :telemetry.detach(handler_id) end)

    config = %{limit: 1, period_ms: 60_000}
    assert :ok = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)
    assert {:error, :rate_limited} = AlchemyRateLimit.hit(@server, :login_ip, "127.0.0.1", config)

    assert_receive {[:alchemy_rate_limit, :rate_limit, :throttle], %{count: 1},
                    %{action: :login, axis: :ip, bucket: :login_ip}}
  end

  test "cleanup does not reset long-period buckets" do
    config = %{limit: 2, period_ms: 3_600_000}

    assert :ok = AlchemyRateLimit.hit(@server, :register_email, "user@example.com", config)
    assert :ok = AlchemyRateLimit.hit(@server, :register_email, "user@example.com", config)

    send(@server, :cleanup)

    assert {:error, :rate_limited} =
             AlchemyRateLimit.hit(@server, :register_email, "user@example.com", config)
  end
end
