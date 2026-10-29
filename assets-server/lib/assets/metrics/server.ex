defmodule Assets.Metrics.Server do
  @moduledoc """
  Loopback-only Bandit for `GET /metrics` (x0049).

  Bind is never the public Phoenix port. Enable with
  `ASSETS_METRICS_ADDR=127.0.0.1:9103` or `ASSETS_METRICS=1` (default addr).
  """

  require Logger

  alias Assets.Metrics

  @default_addr "127.0.0.1:9103"

  def child_spec(_opts) do
    %{
      id: __MODULE__,
      start: {__MODULE__, :start_link, [[]]},
      type: :supervisor,
      # Optional scrape port — bind failure must not take down CAS (host/client parity).
      restart: :temporary,
      shutdown: 5_000
    }
  end

  @spec start_link(keyword()) :: Supervisor.on_start() | :ignore
  def start_link(_opts \\ []) do
    case listen_spec() do
      :disabled ->
        Logger.info(
          "assets metrics off (set ASSETS_METRICS_ADDR=127.0.0.1:9103 or ASSETS_METRICS=1)"
        )

        :ignore

      {:error, reason} ->
        Logger.error("assets metrics disabled: #{reason}")
        :ignore

      {:ok, ip, port} ->
        case Bandit.start_link(
               plug: AssetsWeb.MetricsPlug,
               scheme: :http,
               port: port,
               ip: ip
             ) do
          {:ok, pid} ->
            Logger.info(
              "assets metrics listening on http://#{:inet.ntoa(ip)}:#{port}/metrics (x0049)"
            )

            {:ok, pid}

          {:error, reason} ->
            Logger.error(
              "assets metrics bind failed (CAS continues without /metrics): #{inspect(reason)}"
            )

            :ignore
        end
    end
  end

  @doc false
  def listen_spec do
    cond do
      addr = nonempty_env("ASSETS_METRICS_ADDR") ->
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
    case Assets.Metrics.parse_bind_addr(raw) do
      {:ok, ip, port} -> {:ok, ip, port}
      {:error, reason} -> {:error, reason}
    end
  end
end
