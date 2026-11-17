defmodule Assets.Repo.Migrations.VcsRefsProtected do
  use Ecto.Migration

  @moduledoc """
  x00cc / draft §8·§18.5: per-branch `protected` flag on `vcs_refs`.

  Default false. Protected branches reject direct push and `advance_branch`
  with `:ref_protected` (PR merge is a later slice).
  """

  def up do
    alter table(:vcs_refs) do
      add :protected, :boolean, null: false, default: false
    end
  end

  def down do
    alter table(:vcs_refs) do
      remove :protected
    end
  end
end
