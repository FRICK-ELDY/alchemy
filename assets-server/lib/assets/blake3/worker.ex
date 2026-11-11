defmodule Assets.Blake3.Worker do
  @moduledoc """
  Permanent Port to `blake3-hash --serve` (x0075), pooled under
  `PartitionSupervisor` (x008d / P2-1).

  Nearly the same shape as `World.ManifestSealHash.Worker` (framing / receive /
  reopen). Kept per-app on purpose — no new `shared/` package; diverge only if
  assets vs world contracts need it.

  Guarantees: N OS processes reused for hex digests (`:blake3_hash_partitions`,
  boot-time `System.schedulers_online/0` via runtime.exs / env); `{:packet, 4}`
  framing; up to N concurrent hashes.
  Non-guarantees: more-than-N parallelism; streaming hash API; `/ready` during
  open failure, terminate, or external Port close before handle_info/reopen
  finishes (happy-path reopen publishes the replacement Port first).

  Error surface: `hash_hex/1` **raises** on soft error, timeout, and `:noproc`
  (partition restart / pool down) — upload path fail-closed contract. World seal
  returns `{:error, _}` for HTTP mapping — intentional asymmetry.
  """

  use GenServer

  require Logger

  @partitioner Assets.Blake3.Partitioner

  # Receive waits shorter than GenServer.call so shutdown / caller timeout can win.
  @receive_slack_ms 2_000

  @doc false
  def partitioner_name, do: @partitioner

  @doc false
  def via(key), do: {:via, PartitionSupervisor, {@partitioner, key}}

  def start_link(opts \\ []) when is_list(opts) do
    GenServer.start_link(__MODULE__, opts)
  end

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    timeout = call_timeout_ms()
    # Integer key → rem(abs(key), partitions): unique_integer ≈ round-robin (x008d).
    key = :erlang.unique_integer([:positive])

    case GenServer.call(via(key), {:hash, data}, timeout) do
      {:ok, hex} -> hex
      {:error, reason} -> raise "blake3-hash failed: #{inspect(reason)}"
    end
  catch
    # Upload contract: raise (not {:error,_}). Pool partial restart → :noproc is
    # more likely than single-Port days; still fail-closed for the PUT in flight.
    :exit, {:noproc, _} ->
      raise "blake3-hash failed: :noproc (partition/pool unavailable)"

    :exit, {:timeout, _} ->
      raise "blake3-hash failed: :timeout"
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
        {:ok, Map.put(open_port(partition), :parent, parent)}

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
    clear_published_port(state.partition)
    {:stop, reason, %{state | port: nil}}
  end

  # Stale EXIT from a previous Port after exit_status→reopen, or unknown links:
  # ignore (do not stop). Stop is reserved for parent match above / receive ^parent.
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
        {:reply, {:error, {:exit_status, status}}, reopen(state)}

      {:EXIT, ^port, reason} ->
        {:reply, {:error, {:port_exit, reason}}, reopen(state)}

      # Parent shutdown while blocked in receive: reply then stop.
      {:EXIT, ^parent, reason} when is_pid(parent) ->
        close_port(port)
        clear_published_port(partition)
        {:stop, reason, {:error, {:shutdown, reason}}, %{state | port: nil}}
    after
      timeout ->
        {:reply, {:error, :timeout}, reopen(state)}
    end
  end

  defp open_port(partition) when is_integer(partition) do
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

    publish_port(partition, port)
    %{port: port, partition: partition}
  end

  defp reopen(%{port: old_port, parent: parent, partition: partition}) do
    # Publish the replacement Port first so ready?/0 does not see a nil slot
    # (pool-wide 503) between clear and open. Then close the old OS process.
    new_state = open_port(partition)
    close_port(old_port)
    Map.put(new_state, :parent, parent)
  end

  defp pt_port_key(partition), do: {__MODULE__, :port, partition}

  defp publish_port(partition, port) when is_integer(partition) and is_port(port) do
    :persistent_term.put(pt_port_key(partition), port)
  end

  defp clear_published_port(partition) when is_integer(partition) do
    _ = :persistent_term.erase(pt_port_key(partition))
    :ok
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

    # Prefer in-tree / CARGO_TARGET_DIR over PATH so a stale `cargo install`
    # (pre-x0075, no `--serve`) cannot shadow the protocol build (x008d).
    configured ||
      System.get_env("BLAKE3_HASH_BIN") ||
      protocol_debug_bin() ||
      System.find_executable("blake3-hash") ||
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
