defmodule Assets.CAS do
  @moduledoc """
  Content-addressed storage facade. Backend is `:local` (disk) or `:s3` (R2 / S3-compatible).

  `delete/1` / `delete_orphan/2` are for `Assets.CAS.OrphanGc` (x0074) and test repair.
  Upload compensate must never call them — concurrent putters share the same key.
  """

  alias Assets.CAS.{Local, S3}

  @type exists_result :: true | false | {:error, term()}

  @spec exists?(String.t()) :: exists_result()
  def exists?(blake3_hex), do: backend().exists?(blake3_hex)

  @spec put_if_absent(String.t(), binary()) :: :created | :existing | {:error, term()}
  def put_if_absent(blake3_hex, data), do: backend().put_if_absent(blake3_hex, data)

  @spec read(String.t()) :: {:ok, binary()} | {:error, :not_found | term()}
  def read(blake3_hex), do: backend().read(blake3_hex)

  @doc """
  Remove CAS bytes. Prefer `delete_orphan/2` for GC — shared keys must stay.

  Intended for tests that deliberately break/repair bodies.
  """
  @spec delete(String.t()) :: :ok | {:error, term()}
  def delete(blake3_hex), do: backend().delete(blake3_hex)

  @doc """
  Restore quarantined bytes to live, or drop quarantine if live already exists.

  Used when CasObject is present so an interrupted GC sidecar cannot linger.
  """
  @spec restore_quarantine(String.t()) :: :ok | {:error, term()}
  def restore_quarantine(blake3_hex), do: backend().restore_quarantine(blake3_hex)

  @doc """
  Quarantine → re-check → discard or restore (x0074 TOCTOU fence).

  `still_orphan?/0` must return true only when CasObject is absent and no
  in-flight `quota_reservations` row exists for this blake3.
  """
  @spec delete_orphan(String.t(), (-> boolean())) ::
          :deleted | :aborted | :missing | {:error, term()}
  def delete_orphan(blake3_hex, still_orphan?)
      when is_binary(blake3_hex) and is_function(still_orphan?, 0) do
    case backend().quarantine(blake3_hex) do
      :ok ->
        finish_orphan_delete(blake3_hex, still_orphan?)

      {:error, :not_found} ->
        :missing

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp finish_orphan_delete(blake3_hex, still_orphan?) do
    if still_orphan?.() do
      case backend().discard_quarantine(blake3_hex) do
        :ok -> :deleted
        {:error, reason} -> {:error, reason}
      end
    else
      case backend().restore_quarantine(blake3_hex) do
        :ok -> :aborted
        {:error, reason} -> {:error, reason}
      end
    end
  end

  defp backend do
    case Application.fetch_env!(:assets, :cas_backend) do
      :local -> Local
      :s3 -> S3
      other -> raise "unknown :cas_backend #{inspect(other)} (expected :local or :s3)"
    end
  end
end
