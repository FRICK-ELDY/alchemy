# x0039 / x0075 / x008d: prefer protocol / CARGO_TARGET_DIR over PATH (stale
# cargo install may lack `--serve`). Docker/CI still wins via env / runtime.
protocol_dir = Path.expand("../../protocol", Path.dirname(__ENV__.file))

exe =
  case :os.type() do
    {:win32, _} -> "manifest-seal-hash.exe"
    _ -> "manifest-seal-hash"
  end

cargo_target = System.get_env("CARGO_TARGET_DIR")

candidates =
  [
    System.get_env("MANIFEST_SEAL_HASH_BIN"),
    cargo_target && Path.join([cargo_target, "debug", exe]),
    cargo_target && Path.join([cargo_target, "release", exe]),
    Path.join([protocol_dir, "target", "debug", exe]),
    Path.join([protocol_dir, "target", "release", exe]),
    System.find_executable("manifest-seal-hash")
  ]
  |> Enum.reject(&is_nil/1)

seal_bin = Enum.find(candidates, &File.exists?/1)

seal_bin =
  if seal_bin do
    seal_bin
  else
    {_, 0} =
      System.cmd("cargo", ["build", "--bin", "manifest-seal-hash"],
        cd: protocol_dir,
        stderr_to_stdout: true
      )

    Enum.find(candidates, &File.exists?/1) ||
      raise "manifest-seal-hash not found after cargo build (checked #{inspect(candidates)})"
  end

Application.put_env(:world, :manifest_seal_hash_bin, seal_bin)

ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(World.Repo, :manual)
