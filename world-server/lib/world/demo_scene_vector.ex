defmodule World.DemoSceneVector do
  @moduledoc """
  Shared DemoScene ManifestSeal **test vector** constants (x0079).

  Not used by Room create / seal issuance. Keep in lockstep with
  `alchemy_protocol::demo_scene_vector`. Entries SSoT remains `World.DemoScene`.

  Regen after DemoScene / PROTOCOL_PIN changes (cwd-independent):

      mix run scripts/regen_demo_scene_vector.exs

  Paste printed `blake3=` into `@blake3` here and `DEMO_SCENE_VECTOR_BLAKE3`.
  Paste printed `YAW45_QY` / `YAW45_QW` `from_bits` lines into
  `alchemy_shared_sim::demo` when they change (never copy `@yaw45_*` decimals).
  """

  @room_id "demo-scene-vector"
  @revision 1
  @host_generation 1
  @host_pub_hex String.duplicate("11", 32)
  @host_attestation "x0079-demo-scene-vector-attestation"
  # alchemy_protocol::demo_scene_vector::DEMO_SCENE_VECTOR_BLAKE3
  @blake3 "3413d423d729e11a3f151b6d08bfcc18105e633ef26d58f86afa5059d2093fcb"

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

  @doc "Canonical seal payload map for the shared vector (DemoScene entries)."
  @spec payload() :: map()
  def payload do
    World.CanonicalManifest.build_payload(
      room_id(),
      revision(),
      host_generation(),
      World.DemoScene.build(),
      host_pub(),
      host_attestation()
    )
  end
end
