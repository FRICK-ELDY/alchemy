defmodule Assets.Blake3.Worker do
  @moduledoc """
  Permanent Port to `blake3-hash --serve` (x0075).

  Guarantees: one OS process reused for hex digests; `{:packet, 4}` framing.
  Non-guarantees: parallel hash throughput; zero-fail during Port restart.
  """

  use GenServer

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    timeout = Application.get_env(:assets, :blake3_hash_call_timeout_ms, 30_000)

    case GenServer.call(__MODULE__, {:hash, data}, timeout) do
      {:ok, hex} -> hex
      {:error, reason} -> raise "blake3-hash failed: #{inspect(reason)}"
    end
  end

  @impl true
  def init(_opts) do
    Process.flag(:trap_exit, true)
    {:ok, open_port()}
  end

  @impl true
  def handle_call({:hash, data}, _from, state) do
    timeout = Application.get_env(:assets, :blake3_hash_call_timeout_ms, 30_000)

    case Port.command(state.port, data) do
      true ->
        receive_reply(state, timeout)

      false ->
        {:reply, {:error, :port_command_failed}, reopen(state)}
    end
  end

  @impl true
  def handle_info({port, {:exit_status, _status}}, %{port: port} = state) do
    {:noreply, reopen(state)}
  end

  def handle_info({:EXIT, port, _reason}, %{port: port} = state) do
    {:noreply, reopen(state)}
  end

  def handle_info(_msg, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
    if is_port(state.port) and Port.info(state.port) != nil do
      Port.close(state.port)
    end

    :ok
  end

  defp receive_reply(state, timeout) do
    port = state.port

    receive do
      {^port, {:data, resp}} ->
        {:reply, decode_reply(resp), state}

      {^port, {:exit_status, status}} ->
        {:reply, {:error, {:exit_status, status}}, reopen(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:port_exit, reason}}, reopen(state)}
    after
      timeout ->
        {:reply, {:error, :timeout}, reopen(state)}
    end
  end

  defp open_port do
    path = resolve_executable!(binary_path())

    port =
      Port.open(
        {:spawn_executable, String.to_charlist(path)},
        [
          :binary,
          :exit_status,
          :use_stdio,
          :hide,
          {:packet, 4},
          {:args, ["--serve"]}
        ]
      )

    %{port: port}
  end

  defp reopen(state) do
    if is_port(state.port) do
      try do
        Port.close(state.port)
      rescue
        _ -> :ok
      end
    end

    open_port()
  end

  defp resolve_executable!(path) when is_binary(path) do
    if File.exists?(path) do
      Path.expand(path)
    else
      case System.find_executable(path) do
        nil -> raise "blake3-hash binary not found: #{inspect(path)}"
        abs -> abs
      end
    end
  end

  defp decode_reply(resp) when byte_size(resp) == 64 do
    hex = String.downcase(resp)

    if Regex.match?(~r/^[0-9a-f]{64}$/, hex) do
      {:ok, hex}
    else
      {:error, {:unexpected_hash_output, resp}}
    end
  end

  defp decode_reply(<<"!", reason::binary>>), do: {:error, {:soft_error, reason}}
  defp decode_reply(other), do: {:error, {:unexpected_hash_output, other}}

  defp binary_path do
    configured =
      case Application.get_env(:assets, :blake3_hash_bin) do
        path when is_binary(path) and path != "" -> path
        _ -> nil
      end

    configured ||
      System.get_env("BLAKE3_HASH_BIN") ||
      System.find_executable("blake3-hash") ||
      protocol_debug_bin() ||
      raise "blake3-hash binary not configured (set :blake3_hash_bin or BLAKE3_HASH_BIN)"
  end

  defp protocol_debug_bin do
    exe =
      case :os.type() do
        {:win32, _} -> "blake3-hash.exe"
        _ -> "blake3-hash"
      end

    cargo_target = System.get_env("CARGO_TARGET_DIR")
    project_root = Path.expand("../../../..", :code.lib_dir(:assets))
    protocol_dir = Path.expand("../protocol", project_root)

    candidates =
      [
        cargo_target && Path.join([cargo_target, "debug", exe]),
        cargo_target && Path.join([cargo_target, "release", exe]),
        Path.join([protocol_dir, "target", "debug", exe]),
        Path.join([protocol_dir, "target", "release", exe]),
        Path.expand("../protocol/target/debug/#{exe}", File.cwd!()),
        Path.expand("protocol/target/debug/#{exe}", File.cwd!())
      ]
      |> Enum.reject(&is_nil/1)

    Enum.find(candidates, &File.exists?/1)
  end
end
