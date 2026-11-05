defmodule WorldWeb.Router do
  use WorldWeb, :router

  pipeline :api do
    plug :accepts, ["json"]
  end

  pipeline :user_auth do
    plug WorldWeb.Plugs.AuthenticateUser
  end

  pipeline :rate_limited do
    plug WorldWeb.Plugs.RateLimit
  end

  scope "/api/v1", WorldWeb do
    pipe_through [:api, :user_auth, :rate_limited]

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
    # x006d: host take-out appends mesh entry (canonical scene for re-seal).
    post "/rooms/:room_id/scene/entries", RoomController, :create_scene_entry
    # x006b: PlaceRead — any valid Room Token for the room + verified world_seal.
    post "/rooms/:room_id/place_read", RoomController, :create_place_read
    get "/rooms/:room_id/place_hashes", RoomController, :show_place_hashes
  end

  scope "/", WorldWeb do
    get "/health", HealthController, :index
    get "/.well-known/jwks.json", JwksController, :index
  end
end
