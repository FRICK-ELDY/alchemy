defmodule AssetsWeb.JwksController do
  @moduledoc """
  Publishes SpawnTicket RS256 public keys (x00b5). Private key material never appears here.
  """

  use AssetsWeb, :controller

  alias Assets.SpawnTicket.Keys

  def index(conn, _params) do
    json(conn, Keys.jwks())
  end
end
