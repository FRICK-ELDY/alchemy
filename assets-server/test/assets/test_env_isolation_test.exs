defmodule Assets.TestEnvIsolationTest do
  # async: false so we do not observe a mid-test CAS suite's put_env (x0086).
  # This module is the SSoT for x001e boot cas_path / runtime env ignore.
  use ExUnit.Case, async: false

  alias Assets.CasTmp

  @moduledoc """
  x001e: `config/runtime.exs` must not let compose / developer `.env` override
  test fixtures (CAS → R2, shared DATABASE_PATH, etc.).

  x0086: reaper / orphan GC GenServers stay off in MIX_ENV=test.
  """

  test "cas_backend stays :local in MIX_ENV=test" do
    assert Application.get_env(:assets, :cas_backend) == :local
  end

  test "boot cas_path stays priv/cas_test and ignores ASSETS_CAS_PATH (x001e)" do
    cas_path = Application.get_env(:assets, :cas_path)
    assert cas_path == CasTmp.boot_cas_path()

    compose = System.get_env("ASSETS_CAS_PATH")

    if is_binary(compose) and compose != "" do
      refute cas_path == compose
    end
  end

  test "auth_jwks_fetch_on_start stays false in test" do
    assert Application.get_env(:assets, :auth_jwks_fetch_on_start) == false
  end

  test "test database path is the sqlite fixture not compose DATABASE_PATH" do
    db = Application.get_env(:assets, Assets.Repo)[:database]
    assert is_binary(db)
    assert String.ends_with?(db, "assets_test.sqlite3")
  end

  test "reserved reaper and orphan GC do not start in test" do
    assert Application.get_env(:assets, :quota_reserved_reaper_start) == false
    assert Application.get_env(:assets, :orphan_cas_gc_start) == false
    assert Process.whereis(Assets.Quota.ReservedReaper) == nil
    assert Process.whereis(Assets.CAS.OrphanGc) == nil
  end
end
