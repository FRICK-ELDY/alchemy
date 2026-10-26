defmodule Assets.Blobs do
  @moduledoc """
  Blob upload/download with quota enforcement.
  """

  import Ecto.Query

  alias Assets.Blobs.BlobRef
  alias Assets.CAS
  alias Assets.CAS.S3, as: CasS3
  alias Assets.Quota.QuotaLedger
  alias Assets.Repo

  @spec blake3_hex(binary()) :: String.t()
  def blake3_hex(data) when is_binary(data) do
    Assets.Blake3.hash_hex(data)
  end

  @spec upload(String.t(), binary(), String.t()) ::
          {:ok, BlobRef.t(), :created | :existing} | {:error, term()}
  def upload(owner_sub, data, mime)
      when is_binary(owner_sub) and is_binary(data) and is_binary(mime) do
    max = Application.fetch_env!(:assets, :max_upload_bytes)

    if byte_size(data) > max do
      {:error, :too_large}
    else
      do_upload(owner_sub, data, mime)
    end
  end

  defp do_upload(owner_sub, data, mime) do
    blake3 = blake3_hex(data)
    size = byte_size(data)

    Repo.transaction(fn ->
      case Repo.get(BlobRef, blake3) do
        %BlobRef{} = existing ->
          {:existing, existing}

        nil ->
          insert_new_blob!(owner_sub, blake3, data, size, mime)
      end
    end)
    |> unwrap_upload()
  end

  defp insert_new_blob!(owner_sub, blake3, data, size, mime) do
    ensure_quota!(owner_sub, size)

    case CAS.put_if_absent(blake3, data) do
      :ok ->
        changeset =
          BlobRef.changeset(%BlobRef{}, %{
            blake3: blake3,
            size: size,
            mime: mime_or_default(mime),
            owner_sub: owner_sub
          })

        case Repo.insert(changeset) do
          {:ok, blob} ->
            add_quota_usage!(owner_sub, size)
            {:created, blob}

          {:error, changeset} ->
            Repo.rollback({:invalid, changeset})
        end

      {:error, reason} ->
        Repo.rollback({:cas_error, reason})
    end
  end

  defp unwrap_upload({:ok, {:created, blob}}), do: {:ok, blob, :created}
  defp unwrap_upload({:ok, {:existing, blob}}), do: {:ok, blob, :existing}
  defp unwrap_upload({:error, reason}), do: {:error, reason}

  @spec download(String.t()) ::
          {:ok, binary(), BlobRef.t()} | {:error, :not_found}
  def download(blake3) when is_binary(blake3) do
    with %BlobRef{} = blob <- Repo.get(BlobRef, blake3),
         {:ok, data} <- CAS.read(blake3) do
      {:ok, data, blob}
    else
      nil -> {:error, :not_found}
      {:error, :not_found} -> {:error, :not_found}
      {:error, _} -> {:error, :not_found}
    end
  end

  @doc """
  Resolve blob for HTTP GET: local proxies bytes; s3 returns a signed redirect URL (x001d).
  """
  @spec fetch(String.t()) ::
          {:ok, binary(), BlobRef.t()}
          | {:redirect, String.t(), BlobRef.t()}
          | {:error, :not_found | :presign_failed}
  def fetch(blake3) when is_binary(blake3) do
    case Repo.get(BlobRef, blake3) do
      nil -> {:error, :not_found}
      %BlobRef{} = blob -> fetch_blob(blob, blake3)
    end
  end

  defp fetch_blob(blob, blake3) do
    case Application.fetch_env!(:assets, :cas_backend) do
      :local -> fetch_local(blob, blake3)
      :s3 -> fetch_s3_redirect(blob, blake3)
    end
  end

  defp fetch_local(blob, blake3) do
    case CAS.read(blake3) do
      {:ok, data} -> {:ok, data, blob}
      {:error, _} -> {:error, :not_found}
    end
  end

  defp fetch_s3_redirect(blob, blake3) do
    case CasS3.presign_get(blake3) do
      {:ok, url} -> {:redirect, url, blob}
      {:error, _} -> {:error, :presign_failed}
    end
  end

  @spec quota(String.t()) :: %{used_bytes: non_neg_integer(), limit_bytes: pos_integer()}
  def quota(owner_sub) when is_binary(owner_sub) do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    case Repo.get(QuotaLedger, owner_sub) do
      %QuotaLedger{used_bytes: used, limit_bytes: lim} ->
        %{used_bytes: used, limit_bytes: lim}

      nil ->
        %{used_bytes: 0, limit_bytes: limit}
    end
  end

  defp ensure_quota!(owner_sub, add_bytes) do
    limit = Application.fetch_env!(:assets, :quota_limit_bytes)

    ledger =
      case Repo.get(QuotaLedger, owner_sub) do
        nil ->
          %QuotaLedger{}
          |> QuotaLedger.changeset(%{
            owner_sub: owner_sub,
            used_bytes: 0,
            limit_bytes: limit
          })
          |> Repo.insert!()

        existing ->
          existing
      end

    if ledger.used_bytes + add_bytes > ledger.limit_bytes do
      Repo.rollback(:quota_exceeded)
    else
      :ok
    end
  end

  defp add_quota_usage!(owner_sub, add_bytes) do
    from(q in QuotaLedger, where: q.owner_sub == ^owner_sub)
    |> Repo.update_all(inc: [used_bytes: add_bytes])
  end

  defp mime_or_default(""), do: "application/octet-stream"
  defp mime_or_default(nil), do: "application/octet-stream"
  defp mime_or_default(mime), do: mime
end
