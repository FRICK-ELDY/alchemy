defmodule WorldWeb.ConnCase do
  @moduledoc """
  Conn test helpers for world-server HTTP endpoints.
  """

  use ExUnit.CaseTemplate

  using do
    quote do
      import Plug.Conn
      import Phoenix.ConnTest
      import WorldWeb.ConnCase

      @endpoint WorldWeb.Endpoint
    end
  end

  setup tags do
    World.DataCase.setup_sandbox(tags)
    {:ok, conn: Phoenix.ConnTest.build_conn()}
  end
end
