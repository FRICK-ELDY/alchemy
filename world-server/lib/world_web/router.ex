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

  pipeline :internal do
    plug WorldWeb.Plugs.InternalToken
  end

  scope "/api/v1", WorldWeb do
    pipe_through [:api, :user_auth, :rate_limited]

    get "/rooms", RoomController, :index
    get "/rooms/:room_id", RoomController, :show
    post "/rooms", RoomController, :create
    post "/rooms/:room_id/tokens", RoomController, :create_token
    put "/rooms/:room_id/visibility", RoomController, :update_visibility
    put "/rooms/:room_id/eligible_hosts", RoomController, :update_eligible_hosts
    post "/rooms/:room_id/handover", RoomController, :handover
    post "/rooms/:room_id/handover/accept", RoomController, :accept_handover
    post "/rooms/:room_id/takeover", RoomController, :takeover
    # x00a0: owner kick / manual Room key revoke
    post "/rooms/:room_id/room_key/rotate", RoomController, :rotate_room_key
  end

  # x00a0: auth → world friendship / follow revoke hook
  scope "/internal/v1", WorldWeb do
    pipe_through [:api, :internal]

    post "/relation_changed", RelationController, :changed
  end

  # x0025 / x0026: Bearer is host Room Token (not auth JWT).
  # x0084: operation-separated IP + room_id|jti rate limits (seal / scene / PlaceRead).
  scope "/api/v1", WorldWeb do
    pipe_through [:api, :rate_limited]

    post "/rooms/:room_id/host_keys", RoomController, :create_host_key
    post "/rooms/:room_id/manifest_seal", RoomController, :create_manifest_seal
    get "/rooms/:room_id/scene", RoomController, :show_scene
    # x006d: owner/editor persist mesh to canonical scene.
    post "/rooms/:room_id/scene/entries", RoomController, :create_scene_entry
    # x007b: owner/editor remove canonical mesh.
    delete "/rooms/:room_id/scene/entries/:entity_id", RoomController, :delete_scene_entry
    # x009e: active-host temporary overlay (inventory / session props).
    post "/rooms/:room_id/overlay/entries", RoomController, :create_overlay_entry
    delete "/rooms/:room_id/overlay/entries/:entity_id", RoomController, :delete_overlay_entry
    # x006b: PlaceRead — any valid Room Token for the room + verified world_seal.
    post "/rooms/:room_id/place_read", RoomController, :create_place_read
    get "/rooms/:room_id/place_hashes", RoomController, :show_place_hashes
  end

  scope "/", WorldWeb do
    get "/health", HealthController, :index
    get "/ready", HealthController, :ready
    get "/.well-known/jwks.json", JwksController, :index
  end
end
