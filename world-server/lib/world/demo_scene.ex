defmodule World.DemoScene do
  @moduledoc """
  Default room scene definition (x0027 / P4 minimal + x002c initial TRS / x004d).

  Guarantees: SSoT for demo entries + locomotion assigned at room create;
  `@user_roots` length is the room Presence capacity (contiguous user_root
  entity ids from 3; protocol band ceiling is `USER_ROOT_SLOTS_MAX`;
  spawn pose and tint are paired per entry).
  Non-guarantees: UGC editing; shared-sim; EntityKind taxonomy; collider scale;
  runtime resize of slots after host boot.

  Host unit tests mirror this layout in `demo_entries_for_test` (Rust) —
  keep both in sync manually until a shared fixture exists
  (x0027 / x002c debt: center box yaw45 + sx=1.5 must match the Rust fixture).
  """

  @coin_blake3 "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a"
  @flag_grabbable 1
  @box_half 0.25
  @box_spacing 0.8
  @box_center_z 3.5
  # 45° yaw (xyzw) for center grabbable box — exercises Manifest→Rapier rotation.
  # Mirror constants in host-server `demo_entries_for_test` (x002c).
  @yaw45_qy 0.3826834323650898
  @yaw45_qw 0.9238795325112867

  @box_palette [
    {0.90, 0.35, 0.30},
    {0.90, 0.60, 0.20},
    {0.85, 0.80, 0.25},
    {0.35, 0.75, 0.35},
    {0.25, 0.70, 0.75},
    {0.30, 0.45, 0.90},
    {0.55, 0.35, 0.85},
    {0.85, 0.35, 0.70},
    {0.70, 0.70, 0.75}
  ]

  # Presence capacity = length. Each tuple is {{x,y,z}, {r,g,b}} so spawn/color stay paired (x004d).
  @user_roots [
    {{0.0, 1.6, 6.0}, {0.4, 0.7, 1.0}},
    {{1.5, 1.6, 6.0}, {1.0, 0.55, 0.25}},
    {{-1.5, 1.6, 6.0}, {0.45, 0.9, 0.5}},
    {{3.0, 1.6, 6.0}, {0.9, 0.45, 0.85}}
  ]

  @doc "Fresh demo scene map for a newly created room."
  @spec build() :: map()
  def build do
    %{
      "revision" => 1,
      "locomotion" => %{
        "mode" => "noclip",
        "move_speed" => 4.0
      },
      "entries" => coin_entry() ++ user_root_entries() ++ box_entries()
    }
  end

  defp identity_trs do
    %{
      "qx" => 0.0,
      "qy" => 0.0,
      "qz" => 0.0,
      "qw" => 1.0,
      "sx" => 1.0,
      "sy" => 1.0,
      "sz" => 1.0
    }
  end

  defp coin_entry do
    [
      Map.merge(
        %{
          "entity_id" => 1,
          "blake3" => @coin_blake3,
          "x" => 0.0,
          "y" => 5.0,
          "z" => 0.0,
          "half_extent" => 0.0,
          "flags" => 0,
          "r" => 0.2,
          "g" => 0.9,
          "b" => 0.4,
          "a" => 1.0
        },
        identity_trs()
      )
    ]
  end

  defp user_root_entries do
    Enum.with_index(@user_roots)
    |> Enum.map(fn {{{x, y, z}, {r, g, b}}, i} ->
      Map.merge(
        %{
          "entity_id" => 3 + i,
          "blake3" => "",
          "x" => x,
          "y" => y,
          "z" => z,
          "half_extent" => 0.0,
          "flags" => 0,
          "r" => r,
          "g" => g,
          "b" => b,
          "a" => 1.0
        },
        identity_trs()
      )
    end)
  end

  defp box_entries do
    Enum.with_index(@box_palette)
    |> Enum.map(fn {{r, g, b}, i} ->
      row = div(i, 3)
      col = rem(i, 3)
      x = (col - 1) * @box_spacing
      z = @box_center_z + (row - 1) * @box_spacing

      trs =
        if i == 4 do
          # Center box: yaw 45° + mild X scale (visible Manifest TRS demo).
          %{
            "qx" => 0.0,
            "qy" => @yaw45_qy,
            "qz" => 0.0,
            "qw" => @yaw45_qw,
            "sx" => 1.5,
            "sy" => 1.0,
            "sz" => 1.0
          }
        else
          identity_trs()
        end

      Map.merge(
        %{
          "entity_id" => 10 + i,
          "blake3" => "",
          "x" => x,
          "y" => @box_half,
          "z" => z,
          "half_extent" => @box_half,
          "flags" => @flag_grabbable,
          "r" => r,
          "g" => g,
          "b" => b,
          "a" => 1.0
        },
        trs
      )
    end)
  end
end
