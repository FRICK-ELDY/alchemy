defmodule Assets.Prefab.Payload do
  @moduledoc """
  Prefab logical Tree schema and validation (x005e).

  A Prefab is a logical Tree of child references. Each child has:
  - local_id: stable identifier within this Prefab (non-empty string)
  - blake3: 64-char hex hash of the child blob (mesh etc.)
  - trs: optional Transform {translation[3], rotation[4], scale[3]} (all finite floats)

  The payload is stored as JSON in CAS (access=grant_required).
  Future (x0062): VcsTree has the same logical structure — Assets.Prefab.Payload.from_json/1
  decodes both.

  Guarantees: from_json/1 rejects non-finite TRS, wrong schema_version, missing blake3.
  Non-guarantees: child mesh existence check, semantic TRS normalisation, VcsTree diff.
  """

  @schema_version 1

  @enforce_keys [:schema_version, :children]
  defstruct [:schema_version, :children]

  @type trs :: %{
          translation: [float()],
          rotation: [float()],
          scale: [float()]
        }

  @type child :: %{
          local_id: String.t(),
          blake3: String.t(),
          trs: trs() | nil
        }

  @type t :: %__MODULE__{
          schema_version: pos_integer(),
          children: [child()]
        }

  @doc """
  Serialize Payload struct to JSON binary for CAS storage.
  """
  @spec to_json(t()) :: {:ok, binary()} | {:error, term()}
  def to_json(%__MODULE__{} = payload) do
    Jason.encode(%{
      schema_version: payload.schema_version,
      children: Enum.map(payload.children, &encode_child/1)
    })
  end

  @doc """
  Deserialize and validate from JSON binary. Returns {:error, reason} for any invalid input.
  """
  @spec from_json(binary()) :: {:ok, t()} | {:error, term()}
  def from_json(json) when is_binary(json) do
    with {:ok, map} <- Jason.decode(json) do
      validate(map)
    end
  end

  @doc """
  Build a Payload struct directly (for tests or programmatic use).
  Validates the struct on creation.
  """
  @spec new(pos_integer(), [child()]) :: {:ok, t()} | {:error, term()}
  def new(schema_version, children) when is_integer(schema_version) and is_list(children) do
    # Reuse validate path via round-trip to check children
    validate(%{
      "schema_version" => schema_version,
      "children" => Enum.map(children, &child_to_map/1)
    })
  end

  @doc """
  Validate a decoded JSON map into a Payload struct.
  """
  @spec validate(map()) :: {:ok, t()} | {:error, term()}
  def validate(%{"schema_version" => v, "children" => children})
      when is_integer(v) and v == @schema_version and is_list(children) do
    with {:ok, validated_children} <- validate_children(children) do
      {:ok, %__MODULE__{schema_version: v, children: validated_children}}
    end
  end

  def validate(%{"schema_version" => v}) when is_integer(v) and v != @schema_version do
    {:error, {:unsupported_schema_version, v}}
  end

  def validate(_), do: {:error, :invalid_payload_shape}

  # ---------------------------------------------------------------------------
  # Private
  # ---------------------------------------------------------------------------

  defp validate_children(children) do
    children
    |> Enum.with_index()
    |> Enum.reduce_while({:ok, []}, fn {child, idx}, {:ok, acc} ->
      case validate_child(child) do
        {:ok, c} -> {:cont, {:ok, [c | acc]}}
        {:error, reason} -> {:halt, {:error, {:invalid_child, idx, reason}}}
      end
    end)
    |> case do
      {:ok, reversed} -> {:ok, Enum.reverse(reversed)}
      err -> err
    end
  end

  defp validate_child(%{"local_id" => local_id, "blake3" => blake3} = child)
       when is_binary(local_id) and byte_size(local_id) > 0 and
              is_binary(blake3) and byte_size(blake3) == 64 do
    trs_raw = Map.get(child, "trs")

    with :ok <- validate_trs_optional(trs_raw) do
      {:ok, %{local_id: local_id, blake3: blake3, trs: normalize_trs(trs_raw)}}
    end
  end

  defp validate_child(_), do: {:error, :invalid_child_shape}

  defp validate_trs_optional(nil), do: :ok

  defp validate_trs_optional(%{"translation" => t, "rotation" => r, "scale" => s})
       when is_list(t) and length(t) == 3 and
              is_list(r) and length(r) == 4 and
              is_list(s) and length(s) == 3 do
    all_floats = t ++ r ++ s

    if Enum.all?(all_floats, &finite_numeric?/1) do
      :ok
    else
      {:error, :non_finite_trs}
    end
  end

  defp validate_trs_optional(_), do: {:error, :invalid_trs_shape}

  # Integer values are always finite.
  defp finite_numeric?(v) when is_integer(v), do: true

  # v - v is 0.0 for finite floats; NaN for NaN (NaN-NaN=NaN); NaN for ±Inf (Inf-Inf=NaN).
  # credo:disable-for-next-line Credo.Check.Warning.OperationOnSameValues
  defp finite_numeric?(v) when is_float(v), do: v - v == 0.0
  defp finite_numeric?(_), do: false

  defp normalize_trs(nil), do: nil

  defp normalize_trs(%{"translation" => t, "rotation" => r, "scale" => s}) do
    %{
      translation: Enum.map(t, &to_float/1),
      rotation: Enum.map(r, &to_float/1),
      scale: Enum.map(s, &to_float/1)
    }
  end

  defp to_float(v) when is_integer(v), do: v * 1.0
  defp to_float(v) when is_float(v), do: v

  defp encode_child(%{local_id: lid, blake3: b3, trs: nil}) do
    %{local_id: lid, blake3: b3}
  end

  defp encode_child(%{local_id: lid, blake3: b3, trs: %{} = trs}) do
    %{local_id: lid, blake3: b3, trs: encode_trs(trs)}
  end

  defp encode_trs(%{translation: t, rotation: r, scale: s}) do
    %{translation: t, rotation: r, scale: s}
  end

  defp child_to_map(%{local_id: lid, blake3: b3, trs: nil}) do
    %{"local_id" => lid, "blake3" => b3}
  end

  defp child_to_map(%{local_id: lid, blake3: b3, trs: %{translation: t, rotation: r, scale: s}}) do
    %{
      "local_id" => lid,
      "blake3" => b3,
      "trs" => %{"translation" => t, "rotation" => r, "scale" => s}
    }
  end

  # Incomplete / unexpected child shape → validate_child/1 → {:invalid_child, idx, :invalid_child_shape}.
  defp child_to_map(_), do: %{}
end
