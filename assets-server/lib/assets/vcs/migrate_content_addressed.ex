defmodule Assets.Vcs.MigrateContentAddressed do
  @moduledoc """
  x00c9 cutover hook — **no in-place UUID/`tree_json` conversion**.

  Migration `20261116220000_vcs_content_addressed` **drops** `vcs_commits` /
  `vcs_refs` and rebuilds the content-addressed schema. Any pre-x00c9 VCS rows
  are discarded (dev / hermetic DBs only; production UUID VCS was never shipped).

  `run/0` remains as a documented no-op so ops scripts that call it after Port
  boot do not fail. Draft §17's "re-encode from tree_json" is **not** implemented
  in this slice — fixed as destructive rebuild.
  """

  @doc "No-op after destructive schema rebuild. Always `:ok`."
  @spec run() :: :ok
  def run, do: :ok
end
