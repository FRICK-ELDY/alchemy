defmodule WorldWeb.JwksController do
  use WorldWeb, :controller

  alias World.Token.Keys

  def index(conn, _params) do
    json(conn, Keys.jwks())
  end
end
