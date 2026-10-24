defmodule Assets.DataCase do
  @moduledoc false
  use ExUnit.CaseTemplate

  alias Assets.Repo
  alias Ecto.Adapters.SQL.Sandbox

  using do
    quote do
      alias Assets.Repo
      import Ecto
      import Ecto.Changeset
      import Ecto.Query
      import Assets.DataCase
    end
  end

  setup tags do
    pid = Sandbox.start_owner!(Repo, shared: not tags[:async])
    on_exit(fn -> Sandbox.stop_owner(pid) end)
    :ok
  end
end
