defmodule AssetsWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :assets

  plug Plug.RequestId
  plug Plug.Telemetry, event_prefix: [:phoenix, :endpoint]

  # No Plug.Parsers: POST /blobs uses raw read_body (x002d / P3-3).
  # JSON Content-Type must not consume the body before the controller.
  # No MethodOverride: no body params to supply _method.

  plug Plug.Head
  plug AssetsWeb.Router
end
