defmodule Assets.CAS.S3 do
  @moduledoc """
  S3-compatible CAS (Cloudflare R2, QuObjects, …) keyed by BLAKE3 hex.
  """

  @behaviour Assets.CAS.Backend

  @impl true
  def exists?(blake3_hex) when is_binary(blake3_hex) do
    case head(blake3_hex) do
      :ok -> true
      {:error, :not_found} -> false
      {:error, _reason} -> false
    end
  end

  @impl true
  def put_if_absent(blake3_hex, data) when is_binary(data) do
    case head(blake3_hex) do
      :ok ->
        :ok

      {:error, :not_found} ->
        put(blake3_hex, data)

      {:error, reason} ->
        {:error, reason}
    end
  end

  @impl true
  def read(blake3_hex) when is_binary(blake3_hex) do
    cfg = s3_config!()

    case ExAws.S3.get_object(cfg.bucket, object_key(blake3_hex))
         |> ExAws.request(aws_config(cfg)) do
      {:ok, %{body: body}} when is_binary(body) ->
        {:ok, body}

      {:error, {:http_error, 404, _}} ->
        {:error, :not_found}

      {:error, %{status_code: 404}} ->
        {:error, :not_found}

      {:error, reason} ->
        {:error, reason}
    end
  end

  @spec object_key(String.t()) :: String.t()
  def object_key(blake3_hex) when is_binary(blake3_hex) do
    prefix = s3_config!().prefix
    prefix <> blake3_hex
  end

  defp head(blake3_hex) do
    cfg = s3_config!()

    case ExAws.S3.head_object(cfg.bucket, object_key(blake3_hex))
         |> ExAws.request(aws_config(cfg)) do
      {:ok, _} ->
        :ok

      {:error, {:http_error, 404, _}} ->
        {:error, :not_found}

      {:error, %{status_code: 404}} ->
        {:error, :not_found}

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp put(blake3_hex, data) do
    cfg = s3_config!()

    case ExAws.S3.put_object(cfg.bucket, object_key(blake3_hex), data)
         |> ExAws.request(aws_config(cfg)) do
      {:ok, _} -> :ok
      {:error, reason} -> {:error, reason}
    end
  end

  defp s3_config! do
    Application.fetch_env!(:assets, :cas_s3)
  end

  # Flat keys (host/scheme/port) — ExAws merges these for custom endpoints (R2).
  # Nested `:s3` alone can drop the host and yield broken URLs like `https:/bucket/...`.
  defp aws_config(cfg) do
    [
      access_key_id: cfg.access_key_id,
      secret_access_key: cfg.secret_access_key,
      region: cfg.region,
      json_codec: Jason,
      scheme: "https://",
      host: cfg.host,
      port: 443
    ]
  end
end
