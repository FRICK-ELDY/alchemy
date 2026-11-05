defmodule World.DemoSceneVector do
  @moduledoc """
  Shared DemoScene ManifestSeal test vector (x0079).

  Keep constants in lockstep with `alchemy_protocol::demo_scene_vector`.
  Elixir SSoT for entries is `World.DemoScene`; this module only pins the
  fixed seal fields + expected blake3 used by both Elixir and Rust tests.

  When DemoScene layout changes: update `@blake3` here and
  `DEMO_SCENE_VECTOR_BLAKE3` / `protocol/testdata/demo_scene_seal.json`,
  then align `alchemy_shared_sim::demo` (yaw may need `f64::from_bits`).
  """

  @room_id "demo-scene-vector"
  @revision 1
  @host_generation 1
  @host_pub_hex String.duplicate("11", 32)
  @host_attestation "x0079-demo-scene-vector-attestation"
  # alchemy_protocol::demo_scene_vector::DEMO_SCENE_VECTOR_BLAKE3
  @blake3 "3c3b29b829830ee00a41a384d8f86f193795020bd7b35096ba47e3618f84dc4f"

  @doc "Fixed room_id for the shared vector."
  @spec room_id() :: String.t()
  def room_id, do: @room_id

  @doc "DemoScene revision used in the vector."
  @spec revision() :: non_neg_integer()
  def revision, do: @revision

  @doc "Fixed host_generation for the vector."
  @spec host_generation() :: pos_integer()
  def host_generation, do: @host_generation

  @doc "32-byte host_pub as lowercase hex."
  @spec host_pub_hex() :: String.t()
  def host_pub_hex, do: @host_pub_hex

  @doc "Decode `host_pub_hex/0` to 32 raw bytes."
  @spec host_pub() :: binary()
  def host_pub do
    Base.decode16!(@host_pub_hex, case: :lower)
  end

  @doc "Placeholder attestation string (hash input only)."
  @spec host_attestation() :: String.t()
  def host_attestation, do: @host_attestation

  @doc "Expected canonical ManifestSeal blake3 (lowercase hex)."
  @spec blake3() :: String.t()
  def blake3, do: @blake3
end
