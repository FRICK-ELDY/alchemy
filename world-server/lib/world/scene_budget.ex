defmodule World.SceneBudget do
  @moduledoc """
  Scene / overlay size gates aligned with protocol decode limits (x00a2).

  Mirrors `protocol::decode_limits::{MAX_MANIFEST_ENTRIES, MAX_MANIFEST_BYTES}`.
  Guarantees: next-state JSON entries and byte length are checked before CAS write.
  Non-guarantees: protobuf Manifest wire layout identity; auto-trim of legacy oversize rows.
  """

  # Keep in sync with protocol/src/decode_limits.rs
  @max_entries 4096
  @max_json_bytes 1_048_576

  @type budget_error :: :scene_entries_exceeded | :scene_bytes_exceeded

  @spec max_entries() :: pos_integer()
  def max_entries, do: @max_entries

  @spec max_json_bytes() :: pos_integer()
  def max_json_bytes, do: @max_json_bytes

  @doc "Encode `scene_or_overlay` map after budget check. Returns encoded JSON iodata/binary."
  @spec encode_checked(map()) :: {:ok, String.t()} | {:error, budget_error()}
  def encode_checked(scene) when is_map(scene) do
    entries = entries_of(scene)

    if length(entries) > @max_entries do
      {:error, :scene_entries_exceeded}
    else
      encoded = Jason.encode!(scene)

      if byte_size(encoded) > @max_json_bytes do
        {:error, :scene_bytes_exceeded}
      else
        {:ok, encoded}
      end
    end
  end

  def encode_checked(_), do: {:error, :scene_entries_exceeded}

  @doc """
  Composed canonical ∥ overlay entry count must stay ≤ protocol manifest entry cap.
  """
  @spec ensure_composed(map(), map()) :: :ok | {:error, budget_error()}
  def ensure_composed(canonical, overlay) when is_map(canonical) and is_map(overlay) do
    total = length(entries_of(canonical)) + length(entries_of(overlay))

    if total > @max_entries do
      {:error, :scene_entries_exceeded}
    else
      :ok
    end
  end

  def ensure_composed(_, _), do: {:error, :scene_entries_exceeded}

  defp entries_of(scene) do
    case Map.get(scene, "entries") || Map.get(scene, :entries) do
      list when is_list(list) -> list
      _ -> []
    end
  end
end
