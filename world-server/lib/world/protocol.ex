defmodule World.Protocol do
  @moduledoc """
  Elixir mirror of alchemy-protocol band / pin contracts used by world (x009b).

  Guarantees: `user_root_*` constants match `protocol/src/lib.rs`; Rooms take-out
  id allocation and DemoScene user_root ids share this module (not DemoScene).
  Non-guarantees: full wire schema; PROTO decode limits.

  Lockstep: `World.ProtocolTest` asserts FIRST / MAX against the Rust literals.
  """

  # Mirror alchemy-protocol `USER_ROOT_ENTITY_FIRST` / `USER_ROOT_SLOTS_MAX` (x009b).
  @user_root_entity_first 0x10000000
  @user_root_slots_max 32

  @doc "Protocol user_root band first id (`USER_ROOT_ENTITY_FIRST`)."
  @spec user_root_entity_first() :: non_neg_integer()
  def user_root_entity_first, do: @user_root_entity_first

  @doc "Protocol user_root DoS ceiling (`USER_ROOT_SLOTS_MAX`)."
  @spec user_root_slots_max() :: pos_integer()
  def user_root_slots_max, do: @user_root_slots_max

  @doc "First entity id past the user_root band (exclusive end)."
  @spec user_root_entity_after_band() :: non_neg_integer()
  def user_root_entity_after_band,
    do: @user_root_entity_first + @user_root_slots_max

  @doc "True when `entity_id` is in the Presence user_root band."
  @spec user_root_band?(term()) :: boolean()
  def user_root_band?(entity_id)
      when is_integer(entity_id) and entity_id >= 0 do
    entity_id >= @user_root_entity_first and
      entity_id < @user_root_entity_first + @user_root_slots_max
  end

  def user_root_band?(_), do: false
end
