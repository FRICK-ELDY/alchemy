defmodule World.AuthAccess do
  @moduledoc """
  world → auth S2S for social visibility (x0034).

  Guarantees: HTTP GET `/internal/v1/access` with shared internal token.
  Non-guarantees: caching; JWT-embedded friend lists.
  """

  require Logger

  @spec allowed?(String.t(), String.t(), String.t()) ::
          {:ok, boolean()} | {:error, :unavailable | :invalid_response}
  def allowed?(owner_sub, viewer_sub, visibility)
      when is_binary(owner_sub) and is_binary(viewer_sub) and is_binary(visibility) do
    case Application.get_env(:world, :auth_access_mode, :http) do
      :stub ->
        stub_allowed?(owner_sub, viewer_sub, visibility)

      :http ->
        http_allowed?(owner_sub, viewer_sub, visibility)
    end
  end

  defp stub_allowed?(owner_sub, viewer_sub, visibility) do
    fun = Application.get_env(:world, :auth_access_stub)

    result =
      cond do
        is_function(fun, 3) ->
          fun.(owner_sub, viewer_sub, visibility)

        is_map(fun) ->
          Map.get(fun, {owner_sub, viewer_sub, visibility}, false)

        true ->
          false
      end

    {:ok, result == true}
  end

  defp http_allowed?(owner_sub, viewer_sub, visibility) do
    base = Application.get_env(:world, :auth_internal_url, "http://127.0.0.1:4002")

    token =
      case Application.fetch_env(:world, :auth_internal_token) do
        {:ok, t} when is_binary(t) and t != "" -> t
        _ -> nil
      end

    if is_nil(token) do
      Logger.error("AuthAccess: auth_internal_token unset")
      {:error, :unavailable}
    else
      do_http_allowed(base, token, owner_sub, viewer_sub, visibility)
    end
  end

  defp do_http_allowed(base, token, owner_sub, viewer_sub, visibility) do
    url = String.trim_trailing(base, "/") <> "/internal/v1/access"

    case Req.get(url,
           headers: [{"x-alchemy-internal-token", token}],
           params: %{
             owner_sub: owner_sub,
             viewer_sub: viewer_sub,
             visibility: visibility
           },
           receive_timeout: 3_000
         ) do
      {:ok, %{status: 200, body: %{"allowed" => allowed}}} when is_boolean(allowed) ->
        {:ok, allowed}

      {:ok, %{status: status, body: body}} ->
        Logger.warning("AuthAccess unexpected status=#{status} body=#{inspect(body)}")
        {:error, :invalid_response}

      {:error, reason} ->
        Logger.warning("AuthAccess unavailable: #{inspect(reason)}")
        {:error, :unavailable}
    end
  end
end
