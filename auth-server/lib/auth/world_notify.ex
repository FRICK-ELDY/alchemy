defmodule Auth.WorldNotify do
  @moduledoc """
  auth → world S2S for relation revoke (x00a0).

  Guarantees: fire-and-forget `POST /internal/v1/relation_changed` with shared
  internal token after unfriend / friendship-clearing unfollow (stub mode is sync
  for tests). HTTP uses a short receive timeout and does not block the Social API
  on world latency beyond Task scheduling.
  Non-guarantees: retry queue; world availability; already-issued Room Token JWT revoke.
  """

  require Logger

  @http_timeout_ms 750

  @spec relation_changed(String.t(), String.t(), String.t()) :: :ok
  def relation_changed(sub_a, sub_b, reason)
      when is_binary(sub_a) and is_binary(sub_b) and is_binary(reason) do
    case Application.get_env(:auth, :world_notify_mode, :http) do
      :stub ->
        stub_notify(sub_a, sub_b, reason)

      :http ->
        # Keep Social HTTP responsive: notify off the request path.
        _ =
          Task.Supervisor.start_child(Auth.TaskSupervisor, fn ->
            http_notify(sub_a, sub_b, reason)
          end)

        :ok

      _ ->
        :ok
    end
  end

  def relation_changed(_, _, _), do: :ok

  defp stub_notify(sub_a, sub_b, reason) do
    fun = Application.get_env(:auth, :world_notify_stub)

    cond do
      is_function(fun, 3) ->
        fun.(sub_a, sub_b, reason)
        :ok

      is_pid(fun) ->
        send(fun, {:relation_changed, sub_a, sub_b, reason})
        :ok

      true ->
        :ok
    end
  end

  defp http_notify(sub_a, sub_b, reason) do
    base = Application.get_env(:auth, :world_internal_url)
    token = Application.get_env(:auth, :internal_token)

    cond do
      not is_binary(base) or base == "" ->
        Logger.warning("WorldNotify: world_internal_url unset; skip relation_changed")
        :ok

      not is_binary(token) or token == "" ->
        Logger.warning("WorldNotify: internal_token unset; skip relation_changed")
        :ok

      true ->
        url = String.trim_trailing(base, "/") <> "/internal/v1/relation_changed"

        case Req.post(url,
               headers: [
                 {"x-alchemy-internal-token", token},
                 {"content-type", "application/json"}
               ],
               json: %{sub_a: sub_a, sub_b: sub_b, reason: reason},
               receive_timeout: @http_timeout_ms,
               connect_options: [timeout: @http_timeout_ms]
             ) do
          {:ok, %{status: status}} when status in 200..299 ->
            :ok

          {:ok, %{status: status, body: body}} ->
            Logger.warning("WorldNotify unexpected status=#{status} body=#{inspect(body)}")
            :ok

          {:error, reason} ->
            Logger.warning("WorldNotify unavailable: #{inspect(reason)}")
            :ok
        end
    end
  end
end
