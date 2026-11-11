defmodule World.ManifestSealHash.Worker do
  @moduledoc """
  Permanent Port to `manifest-seal-hash --serve` (x0075), pooled under
  `PartitionSupervisor` (x008d / P2-1).

  Nearly the same shape as `Assets.Blake3.Worker` (framing / receive / reopen).
  Kept per-app on purpose — no new `shared/` package.

  Guarantees: N OS processes reused (`:manifest_seal_hash_partitions`, boot-time
  `System.schedulers_online/0` via runtime.exs / env); same encode path as
  one-shot CLI; `{:packet, 4}`; up to N concurrent seals.
  Non-guarantees: more-than-N parallelism; `/ready` during open failure,
  terminate, or external Port close before handle_info/reopen finishes
  (happy-path reopen publishes the replacement Port first).

  Error surface: returns `{:ok, hex} | {:error, _}` for HTTP mapping in
  `RoomController`. Assets `hash_hex/1` raises instead — intentional facade
  asymmetry (upload vs seal issue path).
  """

  use GenServer

  require Logger

  @partitioner World.ManifestSealHash.Partitioner

  @type hash_result :: {:ok, String.t()} | {:error, term()}

  @receive_slack_ms 2_000

  @doc false
  def partitioner_name, do: @partitioner

  @doc false
  def via(key), do: {:via, PartitionSupervisor, {@partitioner, key}}

  def start_link(opts \\ []) when is_list(opts) do
    GenServer.start_link(__MODULE__, opts)
  end

  @spec blake3_hex(binary()) :: hash_result()
  def blake3_hex(json) when is_binary(json) do
    timeout = call_timeout_ms()
    key = :erlang.unique_integer([:positive])
    GenServer.call(via(key), {:hash, json}, timeout)
  catch
    :exit, {:noproc, _} ->
      {:error, {:cli_missing, "ManifestSealHash.Worker pool not started"}}

    :exit, {:timeout, _} ->
      {:error, {:cli_failed, :timeout, "call timeout"}}
  end

  @doc """
  True when the partitioner is up and **every** partition has an attached OS Port.

  Reads `:persistent_term` + `Port.info/1` — **does not** `GenServer.call`, so a
  long `{:hash, _}` (receive up to ~30s) cannot make `/ready` false-negative (x008b).
  """
  @spec ready?() :: boolean()
  def ready? do
    case Process.whereis(@partitioner) do
      pid when is_pid(pid) ->
        partitions = PartitionSupervisor.partitions(@partitioner)
        partitions > 0 and Enum.all?(0..(partitions - 1), &partition_port_ready?/1)

      nil ->
        false
    end
  end

  defp partition_port_ready?(partition) do
    case :persistent_term.get(pt_port_key(partition), nil) do
      port when is_port(port) -> Port.info(port) != nil
      _ -> false
    end
  end

  @impl true
  def init(opts) do
    Process.flag(:trap_exit, true)
    partition = Keyword.fetch!(opts, :partition)

    case ancestor_parent() do
      parent when is_pid(parent) ->
        case open_port(partition) do
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
    clear_published_port(state.partition)
    {:stop, reason, %{state | port: nil}}
  end

  # Stale EXIT from a previous Port after exit_status→reopen: ignore.
  # Stop only on parent match (idle) or ^parent (receive).
  def handle_info({:EXIT, _pid, _reason}, state), do: {:noreply, state}

  def handle_info(_msg, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
    # Mid-reopen :kill can leave GenServer state on the old Port while PT already
    # holds the replacement — close both so the new OS process is not orphaned.
    state_port = Map.get(state, :port)

    case Map.get(state, :partition) do
      partition when is_integer(partition) ->
        published = :persistent_term.get(pt_port_key(partition), nil)
        close_port(state_port)

        if is_port(published) and published != state_port do
          close_port(published)
        end

        clear_published_port(partition)

      _ ->
        close_port(state_port)
    end

    :ok
  end

  defp receive_reply(%{port: port, parent: parent, partition: partition} = state, timeout) do
    receive do
      {^port, {:data, resp}} ->
        {:reply, decode_reply(resp), state}

      {^port, {:exit_status, status}} ->
        {:reply, {:error, {:cli_failed, status, "port exit"}}, reopen!(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:cli_failed, :exit, inspect(reason)}}, reopen!(state)}

      {:EXIT, ^parent, reason} when is_pid(parent) ->
        close_port(port)
        clear_published_port(partition)

        {:stop, reason, {:error, {:cli_failed, :shutdown, inspect(reason)}}, %{state | port: nil}}
    after
      timeout ->
        {:reply, {:error, {:cli_failed, :timeout, "port reply timeout"}}, reopen!(state)}
    end
  end

  defp open_port(partition) when is_integer(partition) do
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

        publish_port(partition, port)
        {:ok, %{port: port, partition: partition}}
    end
  rescue
    e in ErlangError ->
      {:error, {:cli_missing, Exception.message(e)}}
  end

  defp pt_port_key(partition), do: {__MODULE__, :port, partition}

  defp publish_port(partition, port) when is_integer(partition) and is_port(port) do
    :persistent_term.put(pt_port_key(partition), port)
  end

  defp clear_published_port(partition) when is_integer(partition) do
    _ = :persistent_term.erase(pt_port_key(partition))
    :ok
  end

  defp resolve_executable(path) when is_binary(path) do
    if File.exists?(path) do
      Path.expand(path)
    else
      System.find_executable(path)
    end
  end

  defp resolve_executable(_), do: nil

  defp reopen!(%{port: old_port, parent: parent, partition: partition}) do
    # Publish replacement first so ready?/0 does not see a nil slot mid-reopen.
    case open_port(partition) do
      {:ok, new_state} ->
        close_port(old_port)
        Map.put(new_state, :parent, parent)

      {:error, reason} ->
        # Keep publishing whatever still works; clear only if open failed hard.
        close_port(old_port)
        clear_published_port(partition)
        raise "manifest-seal-hash Port reopen failed: #{inspect(reason)}"
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

    # Prefer in-tree / CARGO_TARGET_DIR over PATH so a stale `cargo install`
    # (pre-x0075, no `--serve`) cannot shadow the protocol build (x008d).
    configured ||
      System.get_env("MANIFEST_SEAL_HASH_BIN") ||
      protocol_debug_bin() ||
      System.find_executable("manifest-seal-hash") ||
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
