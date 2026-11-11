defmodule AssetsWeb.BlobController do
  use AssetsWeb, :controller

  require Logger

  alias Assets.Blobs

  @doc """
  Room / general PUT. Always issues `:owned` BlobGrant (x006f).

  Behaviour is identical to `create_inventory/2` today — only the path differs
  (inventory clients keep a dedicated URL as the x0054 contract).
  """
  def create(conn, _params) do
    read_and_upload(conn)
  end

  @doc """
  Inventory PUT path (x0054 / x006f).

  Behaviour is identical to `create/2` (same `read_and_upload/1`): owned Grant,
  no access header parsing. Path is kept so inventory clients have a stable URL;
  diverge here only when inventory needs a distinct contract again.
  """
  def create_inventory(conn, _params) do
    read_and_upload(conn)
  end

  def show(conn, %{"blake3" => blake3}) do
    opts =
      case get_req_header(conn, "x-alchemy-place-read") do
        [token] when byte_size(token) > 0 -> [place_read: token]
        _ -> []
      end

    case Blobs.fetch(blake3, conn.assigns.auth_sub, opts) do
      {:ok, data, blob} ->
        # x008b / P2-3: never echo CasObject.mime as Content-Type (first-writer
        # squatting / XSS foothold). Display mime stays on Manifest / client.
        conn
        |> put_resp_content_type("application/octet-stream", nil)
        |> put_resp_header("x-content-type-options", "nosniff")
        |> put_resp_header("x-alchemy-blake3", blob.blake3)
        |> send_resp(200, data)

      {:redirect, url, blob} ->
        # nosniff on the 302 itself does not constrain the S3 object Content-Type
        # (non-guarantee / x008b). Harmless echo for intermediaries that copy headers.
        conn
        |> put_resp_header("x-content-type-options", "nosniff")
        |> put_resp_header("x-alchemy-blake3", blob.blake3)
        |> redirect(external: url)

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "not_found"})

      {:error, :forbidden} ->
        conn
        |> put_status(:forbidden)
        |> json(%{error: "forbidden"})

      {:error, :presign_failed} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "presign_failed"})
    end
  end

  defp read_and_upload(conn) do
    max = Application.fetch_env!(:assets, :max_upload_bytes)

    case Plug.Conn.read_body(conn, length: max) do
      {:ok, body, conn} ->
        respond_upload(conn, body)

      {:more, _partial, conn} ->
        conn
        |> put_status(413)
        |> json(%{error: "too_large"})

      {:error, reason} ->
        Logger.warning("blob read_body failed: #{inspect(reason)}")

        conn
        |> put_status(:bad_request)
        |> json(%{error: "read_body_failed"})
    end
  end

  defp respond_upload(conn, body) do
    mime = request_mime(conn)

    case Blobs.upload(conn.assigns.auth_sub, body, mime) do
      {:ok, blob, status} ->
        code = if status == :created, do: :created, else: :ok

        conn
        |> put_status(code)
        |> json(%{
          blake3: blob.blake3,
          size: blob.size,
          mime: blob.mime,
          status: Atom.to_string(status)
        })

      {:error, :quota_exceeded} ->
        conn
        |> put_status(413)
        |> json(%{error: "quota_exceeded"})

      {:error, :too_large} ->
        conn
        |> put_status(413)
        |> json(%{error: "too_large"})

      {:error, {:quarantine, reason}} ->
        conn
        |> put_status(:unprocessable_entity)
        |> json(%{error: "quarantine", reason: Assets.Quarantine.public_reason(reason)})

      {:error, reason} ->
        Logger.warning("blob upload failed: #{inspect(reason)}")

        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "upload_failed"})
    end
  end

  defp request_mime(conn) do
    case get_req_header(conn, "content-type") do
      [value | _] -> value
      _ -> "application/octet-stream"
    end
  end
end
