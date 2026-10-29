defmodule Assets.Blake3 do
  @moduledoc """
  BLAKE3 via the `b3sum` CLI (x004c).

  Guarantees: hex digest of input bytes. On Unix, hashing streams through a FIFO
  (**content** temp file なし; FIFO ノード自体は作る). Windows keeps a short-lived
  content tempfile (Port stdin EOF is unreliable without a pipe NIF).
  Timeout on FIFO hang raises in the caller without linked EXIT（親プロセスは生存）.

  Non-guarantees: in-process Rustler NIF（Hex 承認後）; Windows FIFO parity;
  timeout 時に BEAM worker を `:kill` しても、`System.cmd` 配下の OS 子（b3sum）が
  環境によって一瞬 orphan になり得ること（親落下とは別レイヤ・init が回収）。
  """

  @default_timeout_ms 120_000

  @spec hash_hex(binary()) :: String.t()
  def hash_hex(data) when is_binary(data) do
    b3 = b3sum_path!()

    case :os.type() do
      {:unix, _} -> hash_via_fifo(b3, data)
      {:win32, _} -> hash_via_tempfile(b3, data)
    end
  end

  # Reader + writer are unlinked spawns so :kill / crash never takes down the
  # request process. Both sides share one deadline (writer open can hang too).
  defp hash_via_fifo(b3, data) do
    fifo = fifo_path()
    File.rm(fifo)

    case System.cmd("mkfifo", [fifo], stderr_to_stdout: true) do
      {_, 0} -> :ok
      {out, code} -> raise "mkfifo failed (exit #{code}): #{out}"
    end

    parent = self()
    timeout_ms = timeout_ms()
    deadline = System.monotonic_time(:millisecond) + timeout_ms

    reader =
      spawn(fn ->
        result =
          try do
            {:ok, System.cmd(b3, ["--no-names", fifo], stderr_to_stdout: true)}
          catch
            kind, reason -> {:error, {kind, reason, __STACKTRACE__}}
          end

        send(parent, {:b3_reader, self(), result})
      end)

    writer =
      spawn(fn ->
        result =
          try do
            # Blocks until reader opens the FIFO, then streams + closes ⇒ EOF.
            :ok = File.write(fifo, data)
            {:ok, :written}
          rescue
            e -> {:error, e}
          catch
            kind, reason -> {:error, {kind, reason, __STACKTRACE__}}
          end

        send(parent, {:b3_writer, self(), result})
      end)

    reader_ref = Process.monitor(reader)
    writer_ref = Process.monitor(writer)

    try do
      :ok = await_writer(writer, writer_ref, deadline)
      await_reader(reader, reader_ref, deadline)
    after
      stop_worker(writer, writer_ref)
      stop_worker(reader, reader_ref)
      File.rm(fifo)
    end
  end

  defp await_writer(pid, ref, deadline) do
    case await_message(pid, ref, :b3_writer, deadline) do
      {:ok, :written} ->
        :ok

      {:ok, other} ->
        raise "b3sum FIFO write returned unexpected: #{inspect(other)}"

      {:error, reason} ->
        raise "b3sum FIFO write failed: #{format_reason(reason)}"

      :timeout ->
        raise "b3sum FIFO write timed out after #{timeout_ms()}ms"

      {:down, reason} ->
        raise "b3sum FIFO writer exited: #{inspect(reason)}"
    end
  end

  defp await_reader(pid, ref, deadline) do
    case await_message(pid, ref, :b3_reader, deadline) do
      {:ok, {out, 0}} ->
        out
        |> String.trim()
        |> String.downcase()
        |> validate_hex!()

      {:ok, {out, code}} ->
        raise "b3sum failed (exit #{code}): #{out}. Install with: cargo install b3sum"

      {:error, reason} ->
        raise "b3sum failed: #{format_reason(reason)}"

      :timeout ->
        raise "b3sum timed out after #{timeout_ms()}ms"

      {:down, reason} ->
        raise "b3sum reader exited: #{inspect(reason)}"
    end
  end

  defp await_message(pid, ref, tag, deadline) do
    remaining = deadline - System.monotonic_time(:millisecond)

    if remaining <= 0 do
      :timeout
    else
      receive do
        {^tag, ^pid, {:ok, value}} ->
          Process.demonitor(ref, [:flush])
          {:ok, value}

        {^tag, ^pid, {:error, reason}} ->
          Process.demonitor(ref, [:flush])
          {:error, reason}

        {:DOWN, ^ref, :process, ^pid, reason} ->
          {:down, reason}
      after
        remaining ->
          :timeout
      end
    end
  end

  defp stop_worker(pid, ref) do
    Process.demonitor(ref, [:flush])

    if Process.alive?(pid) do
      Process.exit(pid, :kill)
    end

    flush_worker_mail(pid)
  end

  defp flush_worker_mail(pid) do
    receive do
      {:b3_reader, ^pid, _} -> flush_worker_mail(pid)
      {:b3_writer, ^pid, _} -> flush_worker_mail(pid)
    after
      0 -> :ok
    end
  end

  defp fifo_path do
    Path.join(
      System.tmp_dir!(),
      "alchemy-b3-fifo-#{System.unique_integer([:positive])}"
    )
  end

  defp format_reason(%{__exception__: true} = e), do: Exception.message(e)
  defp format_reason(other), do: inspect(other)

  defp hash_via_tempfile(b3, data) do
    tmp =
      Path.join(
        System.tmp_dir!(),
        "alchemy-b3-#{System.unique_integer([:positive])}"
      )

    try do
      File.write!(tmp, data)

      case System.cmd(b3, ["--no-names", tmp], stderr_to_stdout: true) do
        {out, 0} ->
          out
          |> String.trim()
          |> String.downcase()
          |> validate_hex!()

        {out, code} ->
          raise "b3sum failed (exit #{code}): #{out}. Install with: cargo install b3sum"
      end
    after
      File.rm(tmp)
    end
  end

  # Test hooks: `:blake3_timeout_ms`, `:blake3_executable` (see blake3_test).
  defp timeout_ms do
    Application.get_env(:assets, :blake3_timeout_ms, @default_timeout_ms)
  end

  defp b3sum_path! do
    Application.get_env(:assets, :blake3_executable) ||
      System.find_executable("b3sum") ||
      raise "b3sum not found on PATH. Install with: cargo install b3sum"
  end

  defp validate_hex!(hex) do
    if Regex.match?(~r/^[0-9a-f]{64}$/, hex) do
      hex
    else
      raise "unexpected b3sum output: #{inspect(hex)}"
    end
  end
end
