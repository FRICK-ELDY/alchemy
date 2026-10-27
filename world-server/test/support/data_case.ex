defmodule World.DataCase do
  @moduledoc """
  Setup for tests that touch World.Repo (x0028 SQLite sandbox).
  """

  use ExUnit.CaseTemplate

  alias Ecto.Adapters.SQL.Sandbox
  alias World.Repo

  using do
    quote do
      alias World.Repo

      import Ecto
      import Ecto.Changeset
      import Ecto.Query
      import World.DataCase
    end
  end

  setup tags do
    setup_sandbox(tags)
    :ok
  end

  def setup_sandbox(tags) do
    pid = Sandbox.start_owner!(Repo, shared: not tags[:async])
    on_exit(fn -> Sandbox.stop_owner(pid) end)
  end
end
