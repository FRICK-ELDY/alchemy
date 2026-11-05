defmodule Assets.Blake3.Worker do
  @moduledoc """
  Permanent Port to `blake3-hash --serve` (x0075).

  Nearly the same shape as `World.ManifestSealHash.Worker` (framing / receive /
  reopen). Kept per-app on purpose — no new `shared/` package in x0075; diverge
  only if assets vs world contracts need it.

  Guarantees: one OS process reused for hex digests; `{:packet, 4}` framing.
  Non-guarantees: parallel hash throughput; zero-fail during Port restart.

  Error surface: `hash_hex/1` **raises** (historical `Assets.Blake3` contract →
  upload path). World seal returns `{:error, _}` for HTTP mapping — intentional
  asymmetry, not a Port-layer difference.
  """

  use GenServer

  require Logger

  # Receive waits shorter than GenServer.call so shutdown / caller timeout can win.
  @receive_slack_ms 2_000

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    timeout = call_timeout_ms()

    case GenServer.call(__MODULE__, {:hash, data}, timeout) do
      {:ok, hex} -> hex
      {:error, reason} -> raise "blake3-hash failed: #{inspect(reason)}"
    end
  end

  @impl true
  def init(_opts) do
    Process.flag(:trap_exit, true)

    case ancestor_parent() do
      parent when is_pid(parent) ->
        {:ok, Map.put(open_port(), :parent, parent)}

      _ ->
        Logger.error("Assets.Blake3.Worker: no supervisor parent in $ancestors; refusing start")
        {:stop, :no_parent}
    end
  end

  @impl true
  def handle_call({:hash, data}, _from, state) do
    case Port.command(state.port, data) do
      true ->
        receive_reply(state, receive_timeout_ms())

      false ->
        {:reply, {:error, :port_command_failed}, reopen(state)}
    end
  end

  @impl true
  def handle_info({port, {:exit_status, _status}}, %{port: port} = state) do
    {:noreply, reopen(state)}
  end

  def handle_info({:EXIT, port, _reason}, %{port: port} = state) when is_port(port) do
    {:noreply, reopen(state)}
  end

  def handle_info({:EXIT, parent, reason}, %{parent: parent} = state) when is_pid(parent) do
    close_port(state.port)
    {:stop, reason, %{state | port: nil}}
  end

  # Stale EXIT from a previous Port after exit_status→reopen, or unknown links:
  # ignore (do not stop). Stop is reserved for parent match above / receive ^parent.
  def handle_info({:EXIT, _pid, _reason}, state), do: {:noreply, state}

  def handle_info(_msg, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
    close_port(Map.get(state, :port))
    :ok
  end

  defp receive_reply(%{port: port, parent: parent} = state, timeout) do
    receive do
      {^port, {:data, resp}} ->
        {:reply, decode_reply(resp), state}

      {^port, {:exit_status, status}} ->
        {:reply, {:error, {:exit_status, status}}, reopen(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:port_exit, reason}}, reopen(state)}

      # Parent shutdown while blocked in receive: reply then stop.
      {:EXIT, ^parent, reason} when is_pid(parent) ->
        close_port(port)
        {:stop, reason, {:error, {:shutdown, reason}}, %{state | port: nil}}
    after
      timeout ->
        {:reply, {:error, :timeout}, reopen(state)}
    end
  end

  defp open_port do
    path = resolve_executable!(binary_path())
    max_len = Application.fetch_env!(:assets, :max_upload_bytes)

    port =
      Port.open(
        {:spawn_executable, String.to_charlist(path)},
        [
          :binary,
          :exit_status,
          :use_stdio,
          :hide,
          {:packet, 4},
          {:args, ["--serve"]},
          {:env,
           [
             {~c"BLAKE3_HASH_MAX_LEN", String.to_charlist(Integer.to_string(max_len))}
           ]}
        ]
      )

    %{port: port}
  end

  defp reopen(%{port: port, parent: parent}) do
    close_port(port)
    Map.put(open_port(), :parent, parent)
  end

  defp ancestor_parent do
    case Process.get(:"$ancestors") do
      [parent | _] when is_pid(parent) -> parent
      [name | _] when is_atom(name) -> Process.whereis(name)
      _ -> nil
    end
  end

  defp close_port(port) when is_port(port) do
    if Port.info(port) != nil do
      try do
        Port.close(port)
      rescue
        _ -> :ok
      end
    end

    :ok
  end

  defp close_port(_), do: :ok

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

  defp call_timeout_ms do
    Application.get_env(:assets, :blake3_hash_call_timeout_ms, 30_000)
  end

  defp receive_timeout_ms do
    call = call_timeout_ms()
    # Always strictly shorter than GenServer.call when slack fits; never exceed `call`.
    max(call - @receive_slack_ms, 1)
  end

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
