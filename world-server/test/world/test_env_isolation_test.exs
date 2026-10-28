defmodule World.TestEnvIsolationTest do
  use ExUnit.Case, async: true

  @moduledoc """
  x003b: `config/runtime.exs` must not let compose / developer `.env`
  (`WORLD_DATABASE_PATH`) override the ExUnit SQLite fixture.

  CI (`ci.sh` / `ci-world-local.sh`) sets a non-empty pollution path so this
  test can prove the live Repo database is not that env value.
  """

  test "test database path is the sqlite fixture not compose WORLD_DATABASE_PATH" do
    db = Application.get_env(:world, World.Repo)[:database]
    assert is_binary(db)
    assert String.contains?(db, "world_test")
    assert String.ends_with?(db, ".db")

    case System.get_env("WORLD_DATABASE_PATH") do
      path when is_binary(path) and byte_size(path) > 0 ->
        refute db == path,
               "Repo database must ignore WORLD_DATABASE_PATH=#{inspect(path)} (got #{inspect(db)})"

      _ ->
        # Local `mix test` without pollution env — fixture path alone is the contract.
        :ok
    end
  end

  test "auth_jwks_fetch_on_start stays false in test" do
    assert Application.get_env(:world, :auth_jwks_fetch_on_start) == false
  end

  test "auth_access_mode stays :stub in test" do
    assert Application.get_env(:world, :auth_access_mode) == :stub
  end
end
