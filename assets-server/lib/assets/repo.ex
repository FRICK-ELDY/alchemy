defmodule Assets.Repo do
  use Ecto.Repo,
    otp_app: :assets,
    adapter: Ecto.Adapters.SQLite3
end
