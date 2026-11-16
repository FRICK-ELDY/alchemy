defmodule Assets.VcsObject.Worker do
  @moduledoc """
  Permanent Port to `vcs-object --serve` (x00c9), pooled under PartitionSupervisor.

  Same framing/reopen shape as `Assets.Blake3.Worker`. Request/response are JSON
  (`alchemy_protocol::vcs_object_port`); soft-error prefix is `!`.

  Guarantees: N OS processes; `{:packet, 4}`; fail-closed soft errors as
  `{:error, reason}` (facade does not raise — VCS path maps errors).
  Non-guarantees: more-than-N parallelism; streaming.
  """

  use GenServer

  require Logger

  @partitioner Assets.VcsObject.Partitioner
  @receive_slack_ms 2_000

  @doc false
  def partitioner_name, do: @partitioner

  @doc false
  def via(key), do: {:via, PartitionSupervisor, {@partitioner, key}}

  def start_link(opts \\ []) when is_list(opts) do
    GenServer.start_link(__MODULE__, opts)
  end

  @spec call_json(binary()) :: {:ok, map()} | {:error, term()}
  def call_json(json) when is_binary(json) do
    timeout = call_timeout_ms()
    key = :erlang.unique_integer([:positive])

    case GenServer.call(via(key), {:json, json}, timeout) do
      {:ok, map} -> {:ok, map}
      {:error, reason} -> {:error, reason}
    end
  catch
    :exit, {:noproc, _} ->
      {:error, :noproc}

    :exit, {:timeout, _} ->
      {:error, :timeout}
  end

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
        Logger.error("Assets.VcsObject.Worker: no supervisor parent; refusing start")
        {:stop, :no_parent}
    end
  end

  @impl true
  def handle_call({:json, data}, _from, state) do
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

  def handle_info({:EXIT, _pid, _reason}, state), do: {:noreply, state}
  def handle_info(_msg, state), do: {:noreply, state}

  @impl true
  def terminate(_reason, state) do
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
    max_len = Application.get_env(:assets, :vcs_object_max_len, 1_114_112)

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
             {~c"VCS_OBJECT_MAX_LEN", String.to_charlist(Integer.to_string(max_len))}
           ]}
        ]
      )

    publish_port(partition, port)
    %{port: port, partition: partition}
  end

  defp reopen(%{port: old_port, parent: parent, partition: partition}) do
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
        nil -> raise "vcs-object binary not found: #{inspect(path)}"
        abs -> abs
      end
    end
  end

  defp decode_reply(<<"!", reason::binary>>), do: {:error, {:soft_error, reason}}

  defp decode_reply(resp) when is_binary(resp) do
    case Jason.decode(resp) do
      {:ok, map} when is_map(map) -> {:ok, map}
      {:ok, _} -> {:error, {:unexpected_json, resp}}
      {:error, err} -> {:error, {:json_decode, err}}
    end
  end

  defp call_timeout_ms do
    Application.get_env(:assets, :vcs_object_call_timeout_ms, 30_000)
  end

  defp receive_timeout_ms do
    call = call_timeout_ms()
    max(call - @receive_slack_ms, 1)
  end

  defp binary_path do
    configured =
      case Application.get_env(:assets, :vcs_object_bin) do
        path when is_binary(path) and path != "" -> path
        _ -> nil
      end

    configured ||
      System.get_env("VCS_OBJECT_BIN") ||
      protocol_debug_bin() ||
      System.find_executable("vcs-object") ||
      raise "vcs-object binary not configured (set :vcs_object_bin or VCS_OBJECT_BIN)"
  end

  defp protocol_debug_bin do
    exe =
      case :os.type() do
        {:win32, _} -> "vcs-object.exe"
        _ -> "vcs-object"
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
