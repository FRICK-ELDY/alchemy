defmodule WorldWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :world

  plug Plug.RequestId
  plug Plug.Telemetry, event_prefix: [:phoenix, :endpoint]

  # x0081: stash raw body for host HTTP PoP before JSON decode.
  plug Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json],
    pass: ["*/*"],
    json_decoder: Phoenix.json_library(),
    body_reader: {WorldWeb.CacheBodyReader, :read_body, []}

  plug Plug.MethodOverride
  plug Plug.Head
  plug WorldWeb.Router
end
