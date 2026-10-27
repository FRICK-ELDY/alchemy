defmodule WorldWeb.Router do
  use WorldWeb, :router

  pipeline :api do
    plug :accepts, ["json"]
  end

  pipeline :user_auth do
    plug WorldWeb.Plugs.AuthenticateUser
  end

  scope "/api/v1", WorldWeb do
    pipe_through [:api, :user_auth]

    post "/rooms", RoomController, :create
    post "/rooms/:room_id/tokens", RoomController, :create_token
    put "/rooms/:room_id/visibility", RoomController, :update_visibility
  end

  # x0025 / x0026: Bearer is host Room Token (not auth JWT).
  scope "/api/v1", WorldWeb do
    pipe_through [:api]

    post "/rooms/:room_id/host_keys", RoomController, :create_host_key
    post "/rooms/:room_id/manifest_seal", RoomController, :create_manifest_seal
    get "/rooms/:room_id/scene", RoomController, :show_scene
  end

  scope "/", WorldWeb do
    get "/health", HealthController, :index
    get "/.well-known/jwks.json", JwksController, :index
  end
end
