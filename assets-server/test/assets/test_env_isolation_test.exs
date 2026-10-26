defmodule Assets.TestEnvIsolationTest do
  use ExUnit.Case, async: true

  @moduledoc """
  x001e: `config/runtime.exs` must not let compose / developer `.env` override
  test fixtures (CAS → R2, shared DATABASE_PATH, etc.).
  """

  test "cas_backend stays :local in MIX_ENV=test" do
    assert Application.get_env(:assets, :cas_backend) == :local
  end

  test "cas_path stays the test fixture not compose ASSETS_CAS_PATH" do
    cas_path = Application.get_env(:assets, :cas_path)
    assert is_binary(cas_path)
    assert cas_path == "priv/cas_test"
  end

  test "auth_jwks_fetch_on_start stays false in test" do
    assert Application.get_env(:assets, :auth_jwks_fetch_on_start) == false
  end

  test "test database path is the sqlite fixture not compose DATABASE_PATH" do
    db = Application.get_env(:assets, Assets.Repo)[:database]
    assert is_binary(db)
    assert String.ends_with?(db, "assets_test.sqlite3")
  end
end
