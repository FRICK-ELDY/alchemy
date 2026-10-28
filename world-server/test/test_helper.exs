# x0039: ensure `manifest-seal-hash` is on PATH (Docker image) or built from protocol/.
seal_bin = System.find_executable("manifest-seal-hash")

seal_bin =
  if seal_bin do
    seal_bin
  else
    protocol_dir =
      Path.expand("../../protocol", Path.dirname(__ENV__.file))

    cargo_target =
      case :os.type() do
        {:win32, _} -> "debug\\manifest-seal-hash.exe"
        _ -> "debug/manifest-seal-hash"
      end

    built = Path.join([protocol_dir, "target", cargo_target])

    unless File.exists?(built) do
      {_, 0} =
        System.cmd("cargo", ["build", "--bin", "manifest-seal-hash"],
          cd: protocol_dir,
          stderr_to_stdout: true
        )
    end

    built
  end

Application.put_env(:world, :manifest_seal_hash_bin, seal_bin)

ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(World.Repo, :manual)
