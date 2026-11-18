defmodule AssetsWeb.Plugs.RateLimit do
  @moduledoc """
  Applies per-endpoint rate limits to heavy assets HTTP routes (x00d9).

  Actions: `blob_put` / `vcs_negotiate` / `vcs_push` / `spawn_ticket`
  (IP + `auth_sub`). Must run after `:user_auth`.
  """

  import Plug.Conn
  import Phoenix.Controller

  alias Assets.RateLimit
  alias AssetsWeb.ClientIp

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
        # assets pipelines omit `:accepts`; use `json/2` like AuthenticateUser.
        conn
        |> put_resp_header("retry-after", Integer.to_string(retry_after_seconds(period_ms)))
        |> put_status(:too_many_requests)
        |> json(%{errors: %{detail: "Too Many Requests"}})
        |> halt()
    end
  end

  defp first_exceeded_axis(conn, action, limits) do
    Enum.find_value(rate_limit_keys(conn), fn {axis, key} ->
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

  defp action_for(%{method: "POST", path_info: ["api", "v1", "blobs"]}), do: :blob_put

  defp action_for(%{method: "POST", path_info: ["api", "v1", "inventory", "blobs"]}),
    do: :blob_put

  defp action_for(%{
         method: "POST",
         path_info: ["api", "v1", "vcs", "repos", _repo_id, "negotiate"]
       }),
       do: :vcs_negotiate

  defp action_for(%{method: "POST", path_info: ["api", "v1", "vcs", "repos", _repo_id, "push"]}),
    do: :vcs_push

  defp action_for(%{method: "POST", path_info: ["api", "v1", "spawn_tickets"]}),
    do: :spawn_ticket

  defp action_for(_conn), do: nil

  defp rate_limit_keys(conn) do
    ip = ClientIp.from_conn(conn)

    case conn.assigns[:auth_sub] do
      sub when is_binary(sub) and sub != "" ->
        [{:ip, ip}, {:user, sub}]

      _ ->
        [{:ip, ip}]
    end
  end

  defp bucket(:blob_put, :ip), do: :blob_put_ip
  defp bucket(:blob_put, :user), do: :blob_put_user
  defp bucket(:vcs_negotiate, :ip), do: :vcs_negotiate_ip
  defp bucket(:vcs_negotiate, :user), do: :vcs_negotiate_user
  defp bucket(:vcs_push, :ip), do: :vcs_push_ip
  defp bucket(:vcs_push, :user), do: :vcs_push_user
  defp bucket(:spawn_ticket, :ip), do: :spawn_ticket_ip
  defp bucket(:spawn_ticket, :user), do: :spawn_ticket_user

  defp limits_for(action) do
    Application.get_env(:assets, RateLimit, [])
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
