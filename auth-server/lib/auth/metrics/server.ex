defmodule Auth.Metrics.Server do
  @moduledoc """
  Loopback-only Bandit for `GET /metrics` (x007a).

  Bind is never the public Phoenix port. Enable with
  `AUTH_METRICS_ADDR=127.0.0.1:9104` or `AUTH_METRICS=1` (default addr).
  """

  require Logger

  alias Auth.Metrics

  @default_addr "127.0.0.1:9104"

  def child_spec(_opts) do
    %{
      id: __MODULE__,
      start: {__MODULE__, :start_link, [[]]},
      type: :supervisor,
      restart: :temporary,
      shutdown: 5_000
    }
  end

  @spec start_link(keyword()) :: Supervisor.on_start() | :ignore
  def start_link(_opts \\ []) do
    case listen_spec() do
      :disabled ->
        Logger.info("auth metrics off (set AUTH_METRICS_ADDR=127.0.0.1:9104 or AUTH_METRICS=1)")

        :ignore

      {:error, reason} ->
        Logger.error("auth metrics disabled: #{reason}")
        :ignore

      {:ok, ip, port} ->
        case Bandit.start_link(
               plug: AuthWeb.MetricsPlug,
               scheme: :http,
               port: port,
               ip: ip
             ) do
          {:ok, pid} ->
            Logger.info(
              "auth metrics listening on http://#{:inet.ntoa(ip)}:#{port}/metrics (x007a)"
            )

            {:ok, pid}

          {:error, reason} ->
            Logger.error(
              "auth metrics bind failed (auth continues without /metrics): #{inspect(reason)}"
            )

            :ignore
        end
    end
  end

  @doc false
  def listen_spec do
    cond do
      addr = nonempty_env("AUTH_METRICS_ADDR") ->
        parse_loopback_addr(addr)

      Metrics.enabled_flag?() ->
        parse_loopback_addr(@default_addr)

      true ->
        :disabled
    end
  end

  defp nonempty_env(name) do
    case System.get_env(name) do
      nil -> nil
      "" -> nil
      v -> v
    end
  end

  defp parse_loopback_addr(raw) do
    case Auth.Metrics.parse_bind_addr(raw) do
      {:ok, ip, port} -> {:ok, ip, port}
      {:error, reason} -> {:error, reason}
    end
  end
end
