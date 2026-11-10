defmodule World.RateLimit do
  @moduledoc """
  World-facing wrapper around `AlchemyRateLimit` (x0071).
  """

  @buckets [
    room_create_ip: {:room_create, :ip},
    room_create_user: {:room_create, :user},
    room_token_ip: {:room_token, :ip},
    room_token_user: {:room_token, :user},
    room_visibility_ip: {:room_visibility, :ip},
    room_visibility_user: {:room_visibility, :user},
    # x0084: host Room Token / PlaceRead scope
    host_seal_ip: {:host_seal, :ip},
    host_seal_jti: {:host_seal, :jti},
    host_scene_ip: {:host_scene, :ip},
    host_scene_jti: {:host_scene, :jti},
    place_read_ip: {:place_read, :ip},
    place_read_jti: {:place_read, :jti}
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
          table: :world_rate_limit,
          otp_app: :world,
          config_key: __MODULE__,
          telemetry_prefix: [:world, :rate_limit],
          log_event: "world.rate_limit.throttle",
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
