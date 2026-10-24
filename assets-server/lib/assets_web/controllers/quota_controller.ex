defmodule AssetsWeb.QuotaController do
  use AssetsWeb, :controller

  alias Assets.Blobs

  def show(conn, _params) do
    json(conn, Blobs.quota(conn.assigns.auth_sub))
  end
end
