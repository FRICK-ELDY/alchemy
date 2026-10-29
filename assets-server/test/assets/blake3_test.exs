defmodule Assets.Blake3Test do
  use ExUnit.Case, async: false

  alias Assets.Blake3

  # b3sum empty input (official empty digest).
  @empty_hex "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"

  test "hash_hex empty digest (x004c)" do
    assert Blake3.hash_hex("") == @empty_hex
  end

  test "hash_hex is stable for same bytes" do
    data = :crypto.strong_rand_bytes(4096)
    assert Blake3.hash_hex(data) == Blake3.hash_hex(data)
  end

  test "hash_hex matches known vector for abc" do
    # blake3("abc") — official test vector
    assert Blake3.hash_hex("abc") ==
             "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
  end

  # Unix FIFO only: hangable reader never opens the pipe → writer deadline → raise.
  # Regression: unlink workers so timeout :kill must not EXIT the caller (x004c).
  if match?({:unix, _}, :os.type()) do
    test "fifo timeout raises without killing caller (x004c)" do
      wrapper =
        Path.join(
          System.tmp_dir!(),
          "alchemy-b3-hang-#{System.unique_integer([:positive])}"
        )

      File.write!(wrapper, """
      #!/bin/sh
      # Ignore FIFO path args; never open it so File.write blocks until deadline.
      exec sleep 9999
      """)

      File.chmod!(wrapper, 0o755)

      previous_timeout = Application.get_env(:assets, :blake3_timeout_ms)
      previous_exe = Application.get_env(:assets, :blake3_executable)

      Application.put_env(:assets, :blake3_timeout_ms, 150)
      Application.put_env(:assets, :blake3_executable, wrapper)

      on_exit(fn ->
        File.rm(wrapper)

        if is_nil(previous_timeout) do
          Application.delete_env(:assets, :blake3_timeout_ms)
        else
          Application.put_env(:assets, :blake3_timeout_ms, previous_timeout)
        end

        if is_nil(previous_exe) do
          Application.delete_env(:assets, :blake3_executable)
        else
          Application.put_env(:assets, :blake3_executable, previous_exe)
        end
      end)

      Process.flag(:trap_exit, true)

      assert_raise RuntimeError, ~r/timed out/i, fn ->
        Blake3.hash_hex("hang")
      end

      # Reached ⇒ caller survived; no linked worker EXIT.
      refute_received {:EXIT, _pid, _reason}
      assert Process.alive?(self())
    end
  end
end
