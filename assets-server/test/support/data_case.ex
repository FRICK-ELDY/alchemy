defmodule Assets.DataCase do
  @moduledoc false
  use ExUnit.CaseTemplate

  alias Assets.CasTmp
  alias Assets.Repo
  alias Ecto.Adapters.SQL.Sandbox

  using opts do
    CasTmp.reject_async_cas!(opts)
    cas? = Keyword.get(opts, :cas, false)

    quote do
      alias Assets.Repo
      import Ecto
      import Ecto.Changeset
      import Ecto.Query
      import Assets.DataCase

      # x0086: CAS writers must isolate; forgetting cas: true leaves shared priv/cas_test dirty.
      if unquote(cas?) do
        @moduletag :tmp_dir
      end
    end
  end

  setup tags do
    pid = Sandbox.start_owner!(Repo, shared: not tags[:async])
    on_exit(fn -> Sandbox.stop_owner(pid) end)

    {:ok, CasTmp.install(tags)}
  end
end
