defmodule Auth.AccountsRefreshConcurrencyTest do
  @moduledoc """
  Concurrent refresh needs a shared sandbox (`async: false`) so Task children
  can checkout the Repo (x002d review).

  Note: shared sandbox is one connection, so Tasks are effectively serialized.
  This asserts the API contract (exactly one success) under dual callers, not a
  true FOR UPDATE race. Race safety is in the implementation (TX + lock +
  conditional revoke).
  """

  use Auth.DataCase, async: false

  import Auth.AccountsFixtures

  alias Auth.Accounts

  test "dual refresh callers yield exactly one success" do
    user = user_fixture()
    {:ok, session} = Accounts.issue_session(user, true)
    refresh_token = session.refresh_token

    tasks =
      for _ <- 1..2 do
        Task.async(fn -> Accounts.refresh(refresh_token) end)
      end

    results = Enum.map(tasks, &Task.await(&1, 10_000))
    oks = Enum.count(results, &match?({:ok, _}, &1))
    errs = Enum.count(results, &match?({:error, :invalid_refresh_token}, &1))
    assert oks == 1
    assert errs == 1
  end
end
