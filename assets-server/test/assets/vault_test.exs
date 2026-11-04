defmodule Assets.VaultTest do
  use Assets.DataCase, async: false

  alias Assets.Blobs
  alias Assets.Vault

  @sub "44444444-4444-4444-4444-444444444444"

  setup do
    File.rm_rf!(Application.fetch_env!(:assets, :cas_path))
    File.mkdir_p!(Application.fetch_env!(:assets, :cas_path))
    :ok
  end

  test "personal vault id is provisional owner_sub key" do
    assert Vault.personal_id(@sub) == @sub
  end

  test "personal view matches Blobs.quota for empty ledger" do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    assert Vault.personal(@sub) == %{
             vault_id: @sub,
             kind: :personal,
             used_bytes: 0,
             reserved_bytes: 0,
             limit_bytes: limit
           }

    assert Blobs.quota(@sub) == %{
             used_bytes: 0,
             reserved_bytes: 0,
             limit_bytes: limit
           }
  end

  test "personal view tracks used_bytes after upload" do
    body = "vault-personal-peel"
    assert {:ok, _obj, _status} = Blobs.upload(@sub, body, "text/plain")
    q = Blobs.quota(@sub)
    v = Vault.personal(@sub)

    assert v.vault_id == @sub
    assert v.kind == :personal
    assert v.used_bytes == q.used_bytes
    assert v.reserved_bytes == q.reserved_bytes
    assert v.limit_bytes == q.limit_bytes
    assert v.used_bytes == byte_size(body)
  end
end
