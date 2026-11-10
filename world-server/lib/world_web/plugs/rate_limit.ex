defmodule WorldWeb.Plugs.RateLimit do
  @moduledoc """
  Applies per-endpoint rate limits to world `:user_auth` routes (x0071).

  - `room_create`: IP and authenticated user
  - `room_token`: IP and authenticated user
  - `room_visibility`: IP and authenticated user
  """

  import Plug.Conn
  import Phoenix.Controller

  alias World.RateLimit

  def init(opts), do: opts

  def call(conn, _opts) do
    case action_for(conn) do
      nil -> conn
      action -> enforce(conn, action)
    end
  end

  defp enforce(conn, action) do
    limits = limits_for(action)

    case first_exceeded_axis(conn, action, limits) do
      nil ->
        conn

      {_axis, period_ms} ->
        conn
        |> put_resp_header("retry-after", Integer.to_string(retry_after_seconds(period_ms)))
        |> put_status(:too_many_requests)
        |> put_view(WorldWeb.ErrorJSON)
        |> render(:"429")
        |> halt()
    end
  end

  defp first_exceeded_axis(conn, action, limits) do
    Enum.find_value(rate_limit_keys(conn, action), fn {axis, key} ->
      case check_rate_limit(action, axis, key, limits) do
        {:rate_limited, period_ms} -> {axis, period_ms}
        :ok -> false
      end
    end)
  end

  defp check_rate_limit(action, axis, key, limits) do
    limit_config = Map.fetch!(limits, axis)

    case RateLimit.hit(bucket(action, axis), key, limit_config) do
      :ok -> :ok
      {:error, :rate_limited} -> {:rate_limited, limit_config.period_ms}
    end
  end

  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms"]}), do: :room_create

  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "tokens"]}),
    do: :room_token

  defp action_for(%{method: "PUT", path_info: ["api", "v1", "rooms", _room_id, "visibility"]}),
    do: :room_visibility

  # x0080: reuse visibility budget for host transfer endpoints.
  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "handover"]}),
    do: :room_visibility

  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "takeover"]}),
    do: :room_visibility

  defp action_for(_conn), do: nil

  defp rate_limit_keys(conn, action)
       when action in [:room_create, :room_token, :room_visibility] do
    ip = WorldWeb.ClientIp.from_conn(conn)

    case conn.assigns[:auth_sub] do
      sub when is_binary(sub) and sub != "" ->
        [{:ip, ip}, {:user, sub}]

      _ ->
        [{:ip, ip}]
    end
  end

  defp bucket(:room_create, :ip), do: :room_create_ip
  defp bucket(:room_create, :user), do: :room_create_user
  defp bucket(:room_token, :ip), do: :room_token_ip
  defp bucket(:room_token, :user), do: :room_token_user
  defp bucket(:room_visibility, :ip), do: :room_visibility_ip
  defp bucket(:room_visibility, :user), do: :room_visibility_user

  defp limits_for(action) do
    Application.get_env(:world, RateLimit, [])
    |> Keyword.get(:limits, %{})
    |> Map.fetch!(action)
  end

  defp retry_after_seconds(period_ms) do
    now = System.system_time(:millisecond)
    remaining_ms = period_ms - rem(now, period_ms)

    remaining_ms
    |> div(1000)
    |> max(1)
  end
end
