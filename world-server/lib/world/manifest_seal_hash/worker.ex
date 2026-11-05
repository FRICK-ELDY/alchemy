defmodule World.ManifestSealHash.Worker do
  @moduledoc """
  Permanent Port to `manifest-seal-hash --serve` (x0075).

  Nearly the same shape as `Assets.Blake3.Worker` (framing / receive / reopen).
  Kept per-app on purpose — no new `shared/` package in x0075.

  Guarantees: one OS process reused; same encode path as one-shot CLI; `{:packet, 4}`.
  Non-guarantees: parallel hash throughput; zero-fail during Port restart.

  Error surface: returns `{:ok, hex} | {:error, _}` for HTTP mapping in
  `RoomController`. Assets `hash_hex/1` raises instead — intentional facade
  asymmetry (upload vs seal issue path).
  """

  use GenServer

  require Logger

  @type hash_result :: {:ok, String.t()} | {:error, term()}

  @receive_slack_ms 2_000

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @spec blake3_hex(binary()) :: hash_result()
  def blake3_hex(json) when is_binary(json) do
    timeout = call_timeout_ms()
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

    case ancestor_parent() do
      parent when is_pid(parent) ->
        case open_port() do
          {:ok, state} -> {:ok, Map.put(state, :parent, parent)}
          {:error, reason} -> {:stop, reason}
        end

      _ ->
        Logger.error(
          "World.ManifestSealHash.Worker: no supervisor parent in $ancestors; refusing start"
        )

        {:stop, :no_parent}
    end
  end

  @impl true
  def handle_call({:hash, json}, _from, state) do
    case Port.command(state.port, json) do
      true ->
        receive_reply(state, receive_timeout_ms())

      false ->
        {:reply, {:error, {:cli_failed, :command, "port command failed"}}, reopen!(state)}
    end
  end

  @impl true
  def handle_info({port, {:exit_status, _status}}, %{port: port} = state) do
    {:noreply, reopen!(state)}
  end

  def handle_info({:EXIT, port, _reason}, %{port: port} = state) when is_port(port) do
    {:noreply, reopen!(state)}
  end

  def handle_info({:EXIT, parent, reason}, %{parent: parent} = state) when is_pid(parent) do
    close_port(state.port)
    {:stop, reason, %{state | port: nil}}
  end

  # Stale EXIT from a previous Port after exit_status→reopen: ignore.
  # Stop only on parent match (idle) or ^parent (receive).
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
        {:reply, {:error, {:cli_failed, status, "port exit"}}, reopen!(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:cli_failed, :exit, inspect(reason)}}, reopen!(state)}

      {:EXIT, ^parent, reason} when is_pid(parent) ->
        close_port(port)

        {:stop, reason, {:error, {:cli_failed, :shutdown, inspect(reason)}}, %{state | port: nil}}
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

  defp reopen!(%{port: port, parent: parent}) do
    close_port(port)

    case open_port() do
      {:ok, new_state} -> Map.put(new_state, :parent, parent)
      {:error, reason} -> raise "manifest-seal-hash Port reopen failed: #{inspect(reason)}"
    end
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

  defp call_timeout_ms do
    Application.get_env(:world, :manifest_seal_hash_call_timeout_ms, 30_000)
  end

  defp receive_timeout_ms do
    call = call_timeout_ms()
    max(call - @receive_slack_ms, 1)
  end

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
