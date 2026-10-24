defmodule AssetsWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :assets

  plug Plug.RequestId
  plug Plug.Telemetry, event_prefix: [:phoenix, :endpoint]

  plug Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json],
    pass: ["*/*"],
    json_decoder: Phoenix.json_library(),
    length: 100_000_000

  plug Plug.MethodOverride
  plug Plug.Head
  plug AssetsWeb.Router
end
