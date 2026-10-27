defmodule Assets.CAS.ExAwsReq do
  @moduledoc """
  `ExAws.Request.HttpClient` backed by Req (x002d / P3-6).

  Avoids the `hackney` dependency and its advisories on the S3 CAS path.

  Honored `http_opts`: `:recv_timeout` only (maps to Req `receive_timeout`).
  Other ExAws opts (proxy, follow_redirect, …) are intentionally ignored —
  extend this module when an S3 path needs them.
  """

  @behaviour ExAws.Request.HttpClient

  @impl true
  def request(method, url, body, headers, http_opts) do
    timeout = Keyword.get(http_opts, :recv_timeout, 30_000)

    opts = [
      method: method,
      url: url,
      headers: headers,
      body: body || "",
      receive_timeout: timeout,
      decode_body: false,
      retry: false
    ]

    case Req.request(opts) do
      {:ok, %Req.Response{status: status, headers: resp_headers, body: resp_body}} ->
        {:ok,
         %{
           status_code: status,
           headers: normalize_headers(resp_headers),
           body: resp_body
         }}

      {:error, reason} ->
        {:error, reason}
    end
  end

  defp normalize_headers(headers) when is_map(headers) do
    Enum.flat_map(headers, fn {k, values} ->
      values = List.wrap(values)
      Enum.map(values, fn v -> {to_string(k), to_string(v)} end)
    end)
  end

  defp normalize_headers(headers) when is_list(headers) do
    Enum.map(headers, fn {k, v} -> {to_string(k), to_string(v)} end)
  end
end
