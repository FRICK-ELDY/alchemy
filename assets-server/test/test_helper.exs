# x0075 / x008d / x00c9: prefer protocol / CARGO_TARGET_DIR build over PATH.
protocol_dir = Path.expand("../../protocol", Path.dirname(__ENV__.file))
cargo_target = System.get_env("CARGO_TARGET_DIR")

resolve_bin = fn name ->
  exe =
    case :os.type() do
      {:win32, _} -> "#{name}.exe"
      _ -> name
    end

  env_key =
    case name do
      "blake3-hash" -> "BLAKE3_HASH_BIN"
      "vcs-object" -> "VCS_OBJECT_BIN"
      _ -> nil
    end

  candidates =
    [
      env_key && System.get_env(env_key),
      cargo_target && Path.join([cargo_target, "debug", exe]),
      cargo_target && Path.join([cargo_target, "release", exe]),
      Path.join([protocol_dir, "target", "debug", exe]),
      Path.join([protocol_dir, "target", "release", exe]),
      System.find_executable(name)
    ]
    |> Enum.reject(&is_nil/1)

  case Enum.find(candidates, &File.exists?/1) do
    nil ->
      {_, 0} =
        System.cmd("cargo", ["build", "--bin", name],
          cd: protocol_dir,
          stderr_to_stdout: true
        )

      Enum.find(candidates, &File.exists?/1) ||
        raise "#{name} not found after cargo build (checked #{inspect(candidates)})"

    path ->
      path
  end
end

Application.put_env(:assets, :blake3_hash_bin, resolve_bin.("blake3-hash"))
Application.put_env(:assets, :vcs_object_bin, resolve_bin.("vcs-object"))

ExUnit.start()
Ecto.Adapters.SQL.Sandbox.mode(Assets.Repo, :manual)
