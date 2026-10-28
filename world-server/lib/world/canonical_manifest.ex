defmodule World.CanonicalManifest do
  @moduledoc """
  Builds the canonical ManifestSeal hash input from DB scene + HostAttestation (x0039).

  Guarantees: seal hash is derived from world-owned scene + verified attestation.
  Non-guarantees: host may still assemble the wire Manifest; mismatch → seal denied.
  """

  alias World.HostAttestation
  alias World.ManifestSealHash
  alias World.Rooms

  @doc """
  Verify attestation for `room_id`, load scene, compute canonical blake3.

  Returns `{:ok, revision, host_generation, blake3}` or an error atom/tuple for the controller.
  """
  @spec canonical_blake3(String.t(), String.t()) ::
          {:ok, non_neg_integer(), pos_integer(), String.t()} | {:error, term()}
  def canonical_blake3(room_id, host_attestation)
      when is_binary(room_id) and is_binary(host_attestation) do
    with {:ok, host_pub, host_generation, _claims} <-
           HostAttestation.verify(host_attestation, room_id),
         :ok <- Rooms.match_claimed_host(room_id, host_pub, host_generation),
         {:ok, scene} <- Rooms.scene(room_id),
         {:ok, revision} <- scene_revision(scene),
         payload <-
           build_payload(room_id, revision, host_generation, scene, host_pub, host_attestation),
         {:ok, blake3} <- ManifestSealHash.blake3_hex(payload) do
      {:ok, revision, host_generation, blake3}
    end
  end

  def canonical_blake3(_, _), do: {:error, :invalid_args}

  @doc "Assemble CLI JSON payload (also useful in tests)."
  @spec build_payload(
          String.t(),
          non_neg_integer(),
          pos_integer(),
          map(),
          binary(),
          String.t()
        ) :: map()
  def build_payload(room_id, revision, host_generation, scene, host_pub, host_attestation)
      when is_binary(room_id) and is_integer(revision) and is_integer(host_generation) and
             host_generation >= 1 and is_map(scene) and is_binary(host_pub) and
             is_binary(host_attestation) do
    pin = Application.fetch_env!(:world, :protocol_pin)

    %{
      "room_id" => room_id,
      "revision" => revision,
      "host_generation" => host_generation,
      "protocol_pin" => pin,
      "host_pub_hex" => Base.encode16(host_pub, case: :lower),
      "host_attestation" => host_attestation,
      "locomotion" => normalize_loco(Map.get(scene, "locomotion")),
      "entries" => normalize_entries(Map.get(scene, "entries") || [])
    }
  end

  defp scene_revision(%{"revision" => rev}) when is_integer(rev) and rev >= 0, do: {:ok, rev}

  defp scene_revision(%{"revision" => rev}) when is_binary(rev) do
    case Integer.parse(rev) do
      {n, ""} when n >= 0 -> {:ok, n}
      _ -> {:error, :invalid_scene_revision}
    end
  end

  defp scene_revision(_), do: {:error, :invalid_scene_revision}

  # Match host `DEFAULT_MOVE_SPEED` / DemoScene when locomotion is absent or speed unset.
  @default_move_speed 4.0

  defp normalize_loco(nil), do: %{"mode" => "noclip", "move_speed" => @default_move_speed}

  defp normalize_loco(loco) when is_map(loco) do
    mode =
      case Map.get(loco, "mode") do
        m when is_binary(m) -> String.downcase(m)
        _ -> "noclip"
      end

    # Explicit 0 is kept; missing key matches host default fill for unset loco only.
    speed =
      case Map.fetch(loco, "move_speed") do
        {:ok, s} when is_number(s) -> s * 1.0
        _ -> @default_move_speed
      end

    %{"mode" => mode, "move_speed" => speed}
  end

  defp normalize_entries(entries) when is_list(entries) do
    Enum.map(entries, &normalize_entry/1)
  end

  defp normalize_entry(e) when is_map(e) do
    %{
      "entity_id" => int_field(e, "entity_id", 0),
      "blake3" => string_field(e, "blake3", ""),
      "x" => float_field(e, "x", 0.0),
      "y" => float_field(e, "y", 0.0),
      "z" => float_field(e, "z", 0.0),
      "half_extent" => float_field(e, "half_extent", 0.0),
      "flags" => int_field(e, "flags", 0),
      "r" => float_field(e, "r", 0.0),
      "g" => float_field(e, "g", 0.0),
      "b" => float_field(e, "b", 0.0),
      "a" => float_field(e, "a", 0.0),
      "qx" => float_field(e, "qx", 0.0),
      "qy" => float_field(e, "qy", 0.0),
      "qz" => float_field(e, "qz", 0.0),
      "qw" => float_field(e, "qw", 0.0),
      "sx" => float_field(e, "sx", 0.0),
      "sy" => float_field(e, "sy", 0.0),
      "sz" => float_field(e, "sz", 0.0)
    }
  end

  defp string_field(map, key, default) do
    case Map.get(map, key) do
      v when is_binary(v) -> v
      _ -> default
    end
  end

  defp int_field(map, key, default) do
    case Map.get(map, key) do
      v when is_integer(v) ->
        v

      v when is_float(v) ->
        trunc(v)

      v when is_binary(v) ->
        case Integer.parse(v) do
          {n, ""} -> n
          _ -> default
        end

      _ ->
        default
    end
  end

  defp float_field(map, key, default) do
    case Map.get(map, key) do
      v when is_float(v) ->
        v

      v when is_integer(v) ->
        v * 1.0

      v when is_binary(v) ->
        case Float.parse(v) do
          {n, ""} -> n
          _ -> default
        end

      _ ->
        default
    end
  end
end
