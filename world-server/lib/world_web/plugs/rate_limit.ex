defmodule WorldWeb.Plugs.RateLimit do
  @moduledoc """
  Applies per-endpoint rate limits to world HTTP routes.

  - x0071 `:user_auth` — `room_create` / `room_token` / `room_visibility`（IP + user）
  - x0084 host-token scope — `host_seal` / `host_scene` / `place_read`
    （IP + Bearer claims の `room_id|jti`。path の room_id は使わない）
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

  # x009e: owner allow-list for takeover — same budget as visibility.
  defp action_for(%{
         method: "PUT",
         path_info: ["api", "v1", "rooms", _room_id, "eligible_hosts"]
       }),
       do: :room_visibility

  # x0080: reuse visibility budget for host transfer endpoints.
  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "handover"]}),
    do: :room_visibility

  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "takeover"]}),
    do: :room_visibility

  # x0084: host-token / PlaceRead scope (operation-separated).
  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "host_keys"]}),
    do: :host_seal

  defp action_for(%{
         method: "POST",
         path_info: ["api", "v1", "rooms", _room_id, "manifest_seal"]
       }),
       do: :host_seal

  defp action_for(%{method: "GET", path_info: ["api", "v1", "rooms", _room_id, "scene"]}),
    do: :host_scene

  defp action_for(%{
         method: "POST",
         path_info: ["api", "v1", "rooms", _room_id, "scene", "entries"]
       }),
       do: :host_scene

  defp action_for(%{
         method: "DELETE",
         path_info: ["api", "v1", "rooms", _room_id, "scene", "entries", _entity_id]
       }),
       do: :host_scene

  defp action_for(%{
         method: "POST",
         path_info: ["api", "v1", "rooms", _room_id, "overlay", "entries"]
       }),
       do: :host_scene

  defp action_for(%{
         method: "DELETE",
         path_info: ["api", "v1", "rooms", _room_id, "overlay", "entries", _entity_id]
       }),
       do: :host_scene

  defp action_for(%{method: "POST", path_info: ["api", "v1", "rooms", _room_id, "place_read"]}),
    do: :place_read

  defp action_for(%{method: "GET", path_info: ["api", "v1", "rooms", _room_id, "place_hashes"]}),
    do: :place_read

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

  defp rate_limit_keys(conn, action)
       when action in [:host_seal, :host_scene, :place_read] do
    ip = WorldWeb.ClientIp.from_conn(conn)

    # jti 軸は Bearer claims 由来（path room_id を回してもカウンタを分割できない）。
    # place_read 発行（Room Token）と place_hashes（PlaceRead JWT）は別 jti なので
    # jti 軸は連動せず、家族内の共有床は IP 軸（slice 保証表参照）。
    case peek_bearer_room_jti(conn) do
      {room_id, jti} when is_binary(room_id) and room_id != "" and is_binary(jti) and jti != "" ->
        [{:ip, ip}, {:jti, room_id <> "|" <> jti}]

      {_room_id, jti} when is_binary(jti) and jti != "" ->
        [{:ip, ip}, {:jti, jti}]

      _ ->
        [{:ip, ip}]
    end
  end

  defp peek_bearer_room_jti(conn) do
    with ["Bearer " <> token] <- get_req_header(conn, "authorization"),
         {:ok, claims} when is_map(claims) <- Joken.peek_claims(token) do
      {claim_string(claims, "room_id"), claim_string(claims, "jti")}
    else
      _ -> {nil, nil}
    end
  end

  defp claim_string(claims, key) do
    case Map.get(claims, key) do
      value when is_binary(value) and value != "" -> value
      _ -> nil
    end
  end

  defp bucket(:room_create, :ip), do: :room_create_ip
  defp bucket(:room_create, :user), do: :room_create_user
  defp bucket(:room_token, :ip), do: :room_token_ip
  defp bucket(:room_token, :user), do: :room_token_user
  defp bucket(:room_visibility, :ip), do: :room_visibility_ip
  defp bucket(:room_visibility, :user), do: :room_visibility_user
  defp bucket(:host_seal, :ip), do: :host_seal_ip
  defp bucket(:host_seal, :jti), do: :host_seal_jti
  defp bucket(:host_scene, :ip), do: :host_scene_ip
  defp bucket(:host_scene, :jti), do: :host_scene_jti
  defp bucket(:place_read, :ip), do: :place_read_ip
  defp bucket(:place_read, :jti), do: :place_read_jti

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
