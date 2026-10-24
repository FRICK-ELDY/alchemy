defmodule AssetsWeb.HealthController do
  use AssetsWeb, :controller

  def index(conn, _params) do
    json(conn, %{status: "ok", service: "assets-server", slice: "C-A"})
  end
end
