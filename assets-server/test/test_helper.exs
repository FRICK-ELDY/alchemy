# x0075 / x008d: prefer protocol / CARGO_TARGET_DIR build over PATH (stale cargo
# install may lack `--serve`). Docker/CI still wins via :blake3_hash_bin / env.
protocol_dir = Path.expand("../../protocol", Path.dirname(__ENV__.file))

exe =
  case :os.type() do
    {:win32, _} -> "blake3-hash.exe"
    _ -> "blake3-hash"
  end

cargo_target = System.get_env("CARGO_TARGET_DIR")

candidates =
  [
    System.get_env("BLAKE3_HASH_BIN"),
    cargo_target && Path.join([cargo_target, "debug", exe]),
    cargo_target && Path.join([cargo_target, "release", exe]),
    Path.join([protocol_dir, "target", "debug", exe]),
    Path.join([protocol_dir, "target", "release", exe]),
    System.find_executable("blake3-hash")
  ]
  |> Enum.reject(&is_nil/1)

blake3_bin = Enum.find(candidates, &File.exists?/1)

blake3_bin =
  if blake3_bin do
    blake3_bin
  else
    {_, 0} =
      System.cmd("cargo", ["build", "--bin", "blake3-hash"],
        cd: protocol_dir,
        stderr_to_stdout: true
      )

    Enum.find(candidates, &File.exists?/1) ||
      raise "blake3-hash not found after cargo build (checked #{inspect(candidates)})"
  end

Application.put_env(:assets, :blake3_hash_bin, blake3_bin)

ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(Assets.Repo, :manual)
