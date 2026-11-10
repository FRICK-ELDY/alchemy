defmodule Assets.CasTmpTest do
  @moduledoc """
  Positive hermetic checks for x0086 CAS tmp_dir wiring.
  """
  use Assets.DataCase, async: false, cas: true

  alias Assets.CasTmp

  test "cas: true points Application cas_path at ExUnit tmp_dir", %{cas_path: cas_path} do
    assert is_binary(cas_path)
    assert cas_path == Application.fetch_env!(:assets, :cas_path)
    refute cas_path == CasTmp.boot_cas_path()

    # ExUnit puts tmp_dir under the app `tmp/` tree (not always OS TEMP).
    tmp_root = Path.expand("tmp")
    cas_abs = Path.expand(cas_path)
    rel = Path.relative_to(cas_abs, tmp_root)
    assert rel != cas_abs, "expected under #{tmp_root}, got #{cas_abs}"
    refute String.starts_with?(rel, "..")
  end

  test "CAS writes land under tmp_dir not priv/cas_test", %{cas_path: cas_path} do
    data = "cas-tmp-probe"
    blake3 = Assets.Blobs.blake3_hex(data)
    assert :created = Assets.CAS.put_if_absent(blake3, data)
    assert Assets.CAS.exists?(blake3) == true

    boot_file = Path.join(CasTmp.boot_cas_path(), blake3)
    refute File.exists?(boot_file)

    under_tmp = Path.join(cas_path, blake3)
    assert File.exists?(under_tmp)
  end
end
