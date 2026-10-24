defmodule AssetsWeb.ConnCase do
  @moduledoc """
  Conn test helpers for assets-server HTTP endpoints.
  """

  use ExUnit.CaseTemplate

  alias Assets.Repo
  alias Ecto.Adapters.SQL.Sandbox

  using do
    quote do
      import Plug.Conn
      import Phoenix.ConnTest
      import AssetsWeb.ConnCase

      @endpoint AssetsWeb.Endpoint
    end
  end

  setup tags do
    pid = Sandbox.start_owner!(Repo, shared: not tags[:async])
    on_exit(fn -> Sandbox.stop_owner(pid) end)
    {:ok, conn: Phoenix.ConnTest.build_conn()}
  end
end
