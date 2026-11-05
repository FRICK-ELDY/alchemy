defmodule World.ManifestSealHash.Worker do
  @moduledoc """
  Permanent Port to `manifest-seal-hash --serve` (x0075).

  Guarantees: one OS process reused; same encode path as one-shot CLI; `{:packet, 4}`.
  Non-guarantees: parallel hash throughput; zero-fail during Port restart.
  """

  use GenServer

  @type hash_result :: {:ok, String.t()} | {:error, term()}

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec blake3_hex(binary()) :: hash_result()
  def blake3_hex(json) when is_binary(json) do
    timeout = Application.get_env(:world, :manifest_seal_hash_call_timeout_ms, 30_000)
    GenServer.call(__MODULE__, {:hash, json}, timeout)
  catch
    :exit, {:noproc, _} ->
      {:error, {:cli_missing, "ManifestSealHash.Worker not started"}}

    :exit, {:timeout, _} ->
      {:error, {:cli_failed, :timeout, "call timeout"}}
  end

  @impl true
  def init(_opts) do
    Process.flag(:trap_exit, true)

    case open_port() do
      {:ok, state} -> {:ok, state}
      {:error, reason} -> {:stop, reason}
    end
  end

  @impl true
  def handle_call({:hash, json}, _from, state) do
    timeout = Application.get_env(:world, :manifest_seal_hash_call_timeout_ms, 30_000)

    case Port.command(state.port, json) do
      true ->
        receive_reply(state, timeout)

      false ->
        {:reply, {:error, {:cli_failed, :command, "port command failed"}}, reopen!(state)}
    end
  end

  @impl true
  def handle_info({port, {:exit_status, _status}}, %{port: port} = state) do
    {:noreply, reopen!(state)}
  end

  def handle_info({:EXIT, port, _reason}, %{port: port} = state) do
    {:noreply, reopen!(state)}
  end

  def handle_info(_msg, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
    if Map.get(state, :port) && is_port(state.port) && Port.info(state.port) != nil do
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
        {:reply, {:error, {:cli_failed, status, "port exit"}}, reopen!(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:cli_failed, :exit, inspect(reason)}}, reopen!(state)}
    after
      timeout ->
        {:reply, {:error, {:cli_failed, :timeout, "port reply timeout"}}, reopen!(state)}
    end
  end

  defp open_port do
    case resolve_executable(binary_path()) do
      nil ->
        {:error, {:cli_missing, binary_path()}}

      path ->
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

        {:ok, %{port: port}}
    end
  rescue
    e in ErlangError ->
      {:error, {:cli_missing, Exception.message(e)}}
  end

  defp resolve_executable(path) when is_binary(path) do
    if File.exists?(path) do
      Path.expand(path)
    else
      System.find_executable(path)
    end
  end

  defp resolve_executable(_), do: nil

  defp reopen!(state) do
    if is_port(state.port) do
      try do
        Port.close(state.port)
      rescue
        _ -> :ok
      end
    end

    case open_port() do
      {:ok, new_state} -> new_state
      {:error, reason} -> raise "manifest-seal-hash Port reopen failed: #{inspect(reason)}"
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

  defp decode_reply(<<"!", reason::binary>>),
    do: {:error, {:cli_failed, 1, reason}}

  defp decode_reply(other), do: {:error, {:unexpected_hash_output, other}}

  defp binary_path do
    configured =
      case Application.get_env(:world, :manifest_seal_hash_bin) do
        path when is_binary(path) and path != "" -> path
        _ -> nil
      end

    configured ||
      System.get_env("MANIFEST_SEAL_HASH_BIN") ||
      System.find_executable("manifest-seal-hash") ||
      protocol_debug_bin() ||
      "manifest-seal-hash"
  end

  defp protocol_debug_bin do
    exe =
      case :os.type() do
        {:win32, _} -> "manifest-seal-hash.exe"
        _ -> "manifest-seal-hash"
      end

    cargo_target = System.get_env("CARGO_TARGET_DIR")
    project_root = Path.expand("../../../..", :code.lib_dir(:world))
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
