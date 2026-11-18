defmodule Assets.RateLimit do
  @moduledoc """
  Assets-facing wrapper around `AlchemyRateLimit` (x00d9).
  """

  @buckets [
    blob_put_ip: {:blob_put, :ip},
    blob_put_user: {:blob_put, :user},
    vcs_negotiate_ip: {:vcs_negotiate, :ip},
    vcs_negotiate_user: {:vcs_negotiate, :user},
    vcs_push_ip: {:vcs_push, :ip},
    vcs_push_user: {:vcs_push, :user},
    spawn_ticket_ip: {:spawn_ticket, :ip},
    spawn_ticket_user: {:spawn_ticket, :user}
  ]

  @type limit_config :: AlchemyRateLimit.limit_config()

  def child_spec(opts) do
    %{
      id: __MODULE__,
      start: {__MODULE__, :start_link, [opts]},
      type: :worker,
      restart: :permanent,
      shutdown: 5000
    }
  end

  def start_link(opts \\ []) do
    AlchemyRateLimit.start_link(
      Keyword.merge(
        [
          name: __MODULE__,
          table: :assets_rate_limit,
          otp_app: :assets,
          config_key: __MODULE__,
          telemetry_prefix: [:assets, :rate_limit],
          log_event: "assets.rate_limit.throttle",
          buckets: @buckets
        ],
        opts
      )
    )
  end

  @spec hit(atom(), String.t(), limit_config()) :: :ok | {:error, :rate_limited}
  def hit(bucket, key, config) do
    AlchemyRateLimit.hit(__MODULE__, bucket, key, config)
  end

  @doc false
  def reset do
    AlchemyRateLimit.reset(__MODULE__)
  end
end
