defmodule World.RoomAccess do
  @moduledoc """
  Token-issuance ACL for room visibility (x0034–x0036).

  Owner always allowed. `public` allows any authenticated sub.
  `lan` requires LAN-eligible client path (fail-closed).
  Social levels query auth S2S.

  Non-guarantees: already-issued Room Tokens are not revoked on unfollow /
  unfriend / visibility change; they remain valid until TTL expires.
  """

  alias World.{AuthAccess, Visibility}
  alias WorldWeb.ClientIp

  @spec authorize(Plug.Conn.t(), String.t(), String.t(), String.t()) ::
          :ok
          | {:error, :forbidden_visibility | :lan_required | :auth_unavailable | :not_found}
  def authorize(conn, owner_sub, viewer_sub, visibility)
      when is_binary(owner_sub) and is_binary(viewer_sub) and is_binary(visibility) do
    if owner_sub == viewer_sub do
      :ok
    else
      authorize_visibility(conn, owner_sub, viewer_sub, visibility)
    end
  end

  def authorize(_, _, _, _), do: {:error, :forbidden_visibility}

  defp authorize_visibility(_conn, _owner_sub, _viewer_sub, "public"), do: :ok

  defp authorize_visibility(_conn, _owner_sub, _viewer_sub, "private"),
    do: {:error, :forbidden_visibility}

  defp authorize_visibility(conn, _owner_sub, _viewer_sub, "lan") do
    if ClientIp.lan_eligible?(conn), do: :ok, else: {:error, :lan_required}
  end

  defp authorize_visibility(_conn, owner_sub, viewer_sub, visibility) do
    if Visibility.needs_social?(visibility) do
      social_allowed?(owner_sub, viewer_sub, visibility)
    else
      {:error, :forbidden_visibility}
    end
  end

  defp social_allowed?(owner_sub, viewer_sub, visibility) do
    case AuthAccess.allowed?(owner_sub, viewer_sub, visibility) do
      {:ok, true} -> :ok
      {:ok, false} -> {:error, :forbidden_visibility}
      {:error, _} -> {:error, :auth_unavailable}
    end
  end
end
