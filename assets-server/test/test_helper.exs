# x0075: ensure `blake3-hash` is on PATH (Docker/CI) or built from protocol/.
blake3_bin = System.find_executable("blake3-hash") || System.get_env("BLAKE3_HASH_BIN")

blake3_bin =
  if blake3_bin && File.exists?(blake3_bin) do
    blake3_bin
  else
    protocol_dir = Path.expand("../../protocol", Path.dirname(__ENV__.file))

    exe =
      case :os.type() do
        {:win32, _} -> "blake3-hash.exe"
        _ -> "blake3-hash"
      end

    cargo_target = System.get_env("CARGO_TARGET_DIR")

    candidates =
      [
        cargo_target && Path.join([cargo_target, "debug", exe]),
        cargo_target && Path.join([cargo_target, "release", exe]),
        Path.join([protocol_dir, "target", "debug", exe]),
        Path.join([protocol_dir, "target", "release", exe])
      ]
      |> Enum.reject(&is_nil/1)

    built = Enum.find(candidates, &File.exists?/1)

    built =
      if built do
        built
      else
        {_, 0} =
          System.cmd("cargo", ["build", "--bin", "blake3-hash"],
            cd: protocol_dir,
            stderr_to_stdout: true
          )

        Enum.find(candidates, &File.exists?/1) ||
          raise "blake3-hash not found after cargo build (checked #{inspect(candidates)})"
      end

    built
  end

Application.put_env(:assets, :blake3_hash_bin, blake3_bin)

ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(Assets.Repo, :manual)
