defmodule World.ProtocolTest do
  use ExUnit.Case, async: true

  alias World.Protocol

  test "user_root band constants lockstep with alchemy-protocol (x009b)" do
    # Lockstep with protocol/src/lib.rs (USER_ROOT_ENTITY_FIRST / USER_ROOT_SLOTS_MAX).
    # Rust 変更時は World.Protocol の @attr とこの期待値の両方を更新する。
    assert Protocol.user_root_entity_first() == 0x10000000
    assert Protocol.user_root_slots_max() == 32
    assert Protocol.user_root_entity_after_band() == 0x10000000 + 32
    assert Protocol.user_root_band?(0x10000000)
    assert Protocol.user_root_band?(0x10000000 + 31)
    refute Protocol.user_root_band?(0x10000000 + 32)
    refute Protocol.user_root_band?(3)
    refute Protocol.user_root_band?(6)
  end
end
