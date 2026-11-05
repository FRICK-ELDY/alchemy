defmodule Auth.Metrics do
  @moduledoc """
  Minimal login counters for localhost Prometheus text (x007a / P2-12).

  Guarantees: ETS counters + dedicated loopback Bandit (`AUTH_METRICS_ADDR` or
  `AUTH_METRICS=1` → `127.0.0.1:9104`). Non-guarantees: OTLP / PromEx / public
  scrape / mounting `/metrics` on the Phoenix endpoint.
  """

  @table :auth_metrics

  @keys [
    :login_total,
    :login_errors_total
  ]

  @spec ensure_table() :: :ok
  def ensure_table do
    case :ets.whereis(@table) do
      :undefined ->
        :ets.new(@table, [
          :named_table,
          :public,
          :set,
          read_concurrency: true,
          write_concurrency: true
        ])

        Enum.each(@keys, fn k -> :ets.insert(@table, {k, 0}) end)
        :ok

      _ ->
        :ok
    end
  end

  @doc "True when metrics listener should run (`AUTH_METRICS_ADDR` or `AUTH_METRICS=1`)."
  @spec enabled?() :: boolean()
  def enabled? do
    case System.get_env("AUTH_METRICS_ADDR") do
      v when is_binary(v) and v != "" -> true
      _ -> enabled_flag?()
    end
  end

  @doc false
  @spec enabled_flag?() :: boolean()
  def enabled_flag? do
    System.get_env("AUTH_METRICS") in ["1", "true", "TRUE"]
  end

  @doc """
  Parse `host:port` for metrics bind. Rejects non-loopback (x0049 / x007a).
  """
  @spec parse_bind_addr(String.t()) ::
          {:ok, :inet.ip_address(), :inet.port_number()} | {:error, String.t()}
  def parse_bind_addr(raw) when is_binary(raw) do
    case URI.parse("//" <> raw) do
      %URI{host: host, port: port}
      when is_binary(host) and is_integer(port) and port > 0 and port < 65_536 ->
        resolve_loopback_bind(raw, normalize_metrics_host(host), port)

      _ ->
        {:error, "invalid AUTH_METRICS_ADDR=#{inspect(raw)} (want 127.0.0.1:port)"}
    end
  end

  defp resolve_loopback_bind(raw, host, port) do
    case :inet.parse_address(String.to_charlist(host)) do
      {:ok, ip} -> accept_loopback_ip(raw, ip, port)
      {:error, _} -> {:error, metrics_host_error(host)}
    end
  end

  defp accept_loopback_ip(raw, ip, port) do
    if loopback?(ip) do
      {:ok, ip, port}
    else
      {:error, "AUTH_METRICS_ADDR must be loopback IP (got #{raw})"}
    end
  end

  defp metrics_host_error(host) do
    "invalid metrics host #{inspect(host)} (use 127.0.0.1:port or [::1]:port, not DNS names)"
  end

  defp normalize_metrics_host("localhost"), do: "127.0.0.1"
  defp normalize_metrics_host("LOCALHOST"), do: "127.0.0.1"
  defp normalize_metrics_host(host), do: host

  @spec inc(atom()) :: non_neg_integer()
  def inc(key) when key in @keys do
    ensure_table()
    :ets.update_counter(@table, key, {2, 1}, {key, 0})
  end

  @spec get(atom()) :: non_neg_integer()
  def get(key) when key in @keys do
    ensure_table()

    case :ets.lookup(@table, key) do
      [{^key, v}] when is_integer(v) -> v
      _ -> 0
    end
  end

  @doc "IPv4 127/8, IPv6 ::1, and IPv4-mapped ::ffff:127.x.x.x."
  @spec loopback?(tuple()) :: boolean()
  def loopback?({a, _b, _c, _d}) when is_integer(a) and a == 127, do: true

  def loopback?({0, 0, 0, 0, 0, 0, 0, 1}), do: true

  def loopback?({0, 0, 0, 0, 0, 65_535, hi, lo})
      when is_integer(hi) and is_integer(lo) do
    <<b1, b2, b3, b4>> = <<hi::16, lo::16>>
    loopback?({b1, b2, b3, b4})
  end

  def loopback?(_), do: false

  @spec render() :: String.t()
  def render do
    ensure_table()

    [
      "# HELP alchemy_auth_login_total Login successes\n",
      "# TYPE alchemy_auth_login_total counter\n",
      "alchemy_auth_login_total #{get(:login_total)}\n",
      "# HELP alchemy_auth_login_errors_total Login failures\n",
      "# TYPE alchemy_auth_login_errors_total counter\n",
      "alchemy_auth_login_errors_total #{get(:login_errors_total)}\n"
    ]
    |> IO.iodata_to_binary()
  end
end
