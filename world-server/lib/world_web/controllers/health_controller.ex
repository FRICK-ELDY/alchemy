defmodule WorldWeb.HealthController do
  use WorldWeb, :controller

  def index(conn, _params) do
    json(conn, %{status: "ok", service: "world-server", slice: "B"})
  end
end
