defmodule AssetsWeb.BlobController do
  use AssetsWeb, :controller

  alias Assets.Blobs

  def create(conn, _params) do
    max = Application.fetch_env!(:assets, :max_upload_bytes)

    case Plug.Conn.read_body(conn, length: max) do
      {:ok, body, conn} ->
        respond_upload(conn, body)

      {:more, _partial, conn} ->
        conn
        |> put_status(413)
        |> json(%{error: "too_large"})

      {:error, reason} ->
        conn
        |> put_status(:bad_request)
        |> json(%{error: "read_body_failed", detail: inspect(reason)})
    end
  end

  def show(conn, %{"blake3" => blake3}) do
    case Blobs.fetch(blake3) do
      {:ok, data, blob} ->
        conn
        |> put_resp_content_type(blob.mime)
        |> put_resp_header("x-alchemy-blake3", blob.blake3)
        |> send_resp(200, data)

      {:redirect, url, blob} ->
        conn
        |> put_resp_header("x-alchemy-blake3", blob.blake3)
        |> redirect(external: url)

      {:error, :not_found} ->
        conn
        |> put_status(:not_found)
        |> json(%{error: "not_found"})

      {:error, :presign_failed} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "presign_failed"})
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

      {:error, reason} ->
        conn
        |> put_status(:internal_server_error)
        |> json(%{error: "upload_failed", detail: inspect(reason)})
    end
  end

  defp request_mime(conn) do
    case get_req_header(conn, "content-type") do
      [value | _] -> value
      _ -> "application/octet-stream"
    end
  end
end
