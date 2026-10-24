defmodule AssetsWeb.Router do
  use AssetsWeb, :router

  pipeline :user_auth do
    plug AssetsWeb.Plugs.AuthenticateUser
  end

  scope "/api/v1", AssetsWeb do
    pipe_through [:user_auth]

    post "/blobs", BlobController, :create
    get "/blobs/:blake3", BlobController, :show
    get "/quota", QuotaController, :show
  end

  scope "/", AssetsWeb do
    get "/health", HealthController, :index
  end
end
