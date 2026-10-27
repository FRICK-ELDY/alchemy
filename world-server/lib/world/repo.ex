defmodule World.Repo do
  @moduledoc """
  Persistence for world-server (x0028).

  Default: AshSqlite. Operators can later swap to AshPostgres.Repo +
  `AshPostgres.DataLayer` on resources without changing Room attributes
  (`:string` / `:map` / timestamps only).
  """

  use AshSqlite.Repo,
    otp_app: :world

  @impl true
  def min_sqlite_version do
    %Version{major: 3, minor: 45, patch: 0}
  end
end
