defmodule Auth.RateLimit do
  @moduledoc """
  Auth-facing wrapper around `AlchemyRateLimit` (x0071).

  Keeps the previous `hit/3` / `reset/0` API used by `AuthWeb.Plugs.RateLimit`.
  """

  @buckets [
    login_ip: {:login, :ip},
    login_identifier: {:login, :identifier},
    register_ip: {:register, :ip},
    register_email: {:register, :email},
    refresh_ip: {:refresh, :ip},
    refresh_token: {:refresh, :token},
    resend_verification_ip: {:resend_verification, :ip},
    resend_verification_email: {:resend_verification, :email},
    forgot_password_ip: {:forgot_password, :ip},
    forgot_password_email: {:forgot_password, :email},
    reset_password_ip: {:reset_password, :ip},
    verify_email_ip: {:verify_email, :ip},
    social_mutate_ip: {:social_mutate, :ip},
    social_mutate_user: {:social_mutate, :user}
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
          table: :auth_rate_limit,
          otp_app: :auth,
          config_key: __MODULE__,
          telemetry_prefix: [:auth, :rate_limit],
          log_event: "auth.rate_limit.throttle",
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
