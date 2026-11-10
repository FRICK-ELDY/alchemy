defmodule AssetsWeb.ConnCase do
  @moduledoc """
  Conn test helpers for assets-server HTTP endpoints.
  """

  use ExUnit.CaseTemplate

  alias Assets.CasTmp
  alias Assets.Repo
  alias Ecto.Adapters.SQL.Sandbox

  using opts do
    CasTmp.reject_async_cas!(opts)
    cas? = Keyword.get(opts, :cas, false)

    quote do
      import Plug.Conn
      import Phoenix.ConnTest
      import AssetsWeb.ConnCase

      @endpoint AssetsWeb.Endpoint

      # x0086: see Assets.CasTmp — CAS HTTP suites need async: false + cas: true.
      if unquote(cas?) do
        @moduletag :tmp_dir
      end
    end
  end

  setup tags do
    pid = Sandbox.start_owner!(Repo, shared: not tags[:async])
    on_exit(fn -> Sandbox.stop_owner(pid) end)

    {:ok, Map.merge(%{conn: Phoenix.ConnTest.build_conn()}, CasTmp.install(tags))}
  end
end
