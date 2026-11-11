defmodule AssetsWeb.Router do
  use AssetsWeb, :router

  pipeline :user_auth do
    plug AssetsWeb.Plugs.AuthenticateUser
  end

  scope "/api/v1", AssetsWeb do
    pipe_through [:user_auth]

    post "/blobs", BlobController, :create
    # x0054 / x006f: inventory path alias (same upload as /blobs; owned Grant always).
    post "/inventory/blobs", BlobController, :create_inventory
    get "/blobs/:blake3", BlobController, :show
    get "/quota", QuotaController, :show
    # x006d / x006e: Drive list + take-out resolve + put-back save_prefab.
    get "/drive/nodes", DriveController, :index
    post "/drive/take_out", DriveController, :take_out
    post "/drive/put_back", DriveController, :put_back
  end

  scope "/", AssetsWeb do
    get "/health", HealthController, :index
    get "/ready", HealthController, :ready
  end
end
