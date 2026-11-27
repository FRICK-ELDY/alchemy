defmodule Auth.WorldNotify do
  @moduledoc """
  auth → world S2S for relation revoke (x00a0 / **x00d7**).

  Guarantees: social mutate commits a durable `relation_outbox` row in the same TX;
  stub mode delivers synchronously after commit; HTTP mode is drained by
  `Auth.WorldNotify.Dispatcher` until world returns 2xx (idempotent `event_id`).
  Non-guarantees: already-issued Room Token JWT revoke; world availability latency bound.
  """

  alias Auth.WorldNotify.Dispatcher
  alias Auth.WorldNotify.Outbox

  @http_timeout_ms 750

  @doc """
  After a successful social TX that inserted `row`, deliver (stub) or kick HTTP dispatcher.
  """
  @spec after_commit(Outbox.t()) :: :ok
  def after_commit(%Outbox{} = row) do
    case Application.get_env(:auth, :world_notify_mode, :http) do
      :stub ->
        stub_notify(Outbox.payload(row))
        _ = Outbox.mark_delivered(row)
        :ok

      :http ->
        Dispatcher.kick()
        :ok

      _ ->
        :ok
    end
  end

  @doc false
  @spec http_deliver(map()) :: :ok | {:error, term()}
  def http_deliver(
        %{event_id: _, sub_a: _, sub_b: _, reason: _, affected_owner_subs: _} = payload
      ) do
    base = Application.get_env(:auth, :world_internal_url)
    token = Application.get_env(:auth, :internal_token)

    cond do
      not is_binary(base) or base == "" ->
        {:error, :world_internal_url_unset}

      not is_binary(token) or token == "" ->
        {:error, :internal_token_unset}

      true ->
        url = String.trim_trailing(base, "/") <> "/internal/v1/relation_changed"

        case Req.post(url,
               headers: [
                 {"x-alchemy-internal-token", token},
                 {"content-type", "application/json"}
               ],
               json: payload,
               receive_timeout: @http_timeout_ms,
               connect_options: [timeout: @http_timeout_ms]
             ) do
          {:ok, %{status: status}} when status in 200..299 ->
            :ok

          {:ok, %{status: status, body: body}} ->
            {:error, {:unexpected_status, status, body}}

          {:error, reason} ->
            {:error, reason}
        end
    end
  end

  defp stub_notify(%{
         event_id: event_id,
         sub_a: sub_a,
         sub_b: sub_b,
         reason: reason,
         affected_owner_subs: owners
       }) do
    fun = Application.get_env(:auth, :world_notify_stub)

    cond do
      is_function(fun, 1) ->
        fun.(%{
          event_id: event_id,
          sub_a: sub_a,
          sub_b: sub_b,
          reason: reason,
          affected_owner_subs: owners
        })

        :ok

      is_function(fun, 3) ->
        # Legacy x00a0 stub arity — keep for older tests.
        fun.(sub_a, sub_b, reason)
        :ok

      is_pid(fun) ->
        send(fun, {:relation_changed, event_id, sub_a, sub_b, reason, owners})
        :ok

      true ->
        :ok
    end
  end
end
