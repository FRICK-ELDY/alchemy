defmodule WorldWeb.Router do
  use WorldWeb, :router

  pipeline :api do
    plug :accepts, ["json"]
  end

  scope "/api/v1", WorldWeb do
    pipe_through :api

    post "/rooms", RoomController, :create
    post "/rooms/:room_id/tokens", RoomController, :create_token
  end

  scope "/", WorldWeb do
    get "/health", HealthController, :index
    get "/.well-known/jwks.json", JwksController, :index
  end
end
