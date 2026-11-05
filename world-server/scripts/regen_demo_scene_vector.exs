# Regen DemoScene shared seal vector (x0079).
#
# cwd は問わない（出力先は本ファイル位置基準）:
#   mix run scripts/regen_demo_scene_vector.exs
#
# 1. protocol/testdata/demo_scene_seal.json を上書き
# 2. blake3= を stdout に出す（@blake3 / DEMO_SCENE_VECTOR_BLAKE3 用）
# 3. cargo があれば center box の paste-ready YAW45 from_bits も出す
#    （alchemy_shared_sim::demo。Elixir @yaw45_* 小数は使わない）

alias World.DemoSceneVector
alias World.ManifestSealHash

payload = DemoSceneVector.payload()
{:ok, blake3} = ManifestSealHash.blake3_hex(payload)

# scripts/ → world-server/ → repo root
repo_root = Path.expand("../..", __DIR__)
protocol_dir = Path.join(repo_root, "protocol")
out = Path.join(protocol_dir, "testdata/demo_scene_seal.json")
File.mkdir_p!(Path.dirname(out))
File.write!(out, Jason.encode!(payload, pretty: true) <> "\n")

IO.puts("wrote=#{out}")
IO.puts("blake3=#{blake3}")
IO.puts("")

cargo_toml = Path.join(protocol_dir, "Cargo.toml")

yaw =
  case System.find_executable("cargo") do
    nil ->
      {:error, :cargo_not_found}

    cargo ->
      System.cmd(
        cargo,
        [
          "run",
          "-q",
          "--manifest-path",
          cargo_toml,
          "--bin",
          "demo-scene-vector-yaw-bits",
          "--",
          out
        ],
        stderr_to_stdout: true
      )
  end

case yaw do
  {yaw_out, 0} ->
    IO.puts(String.trim_trailing(yaw_out))
    IO.puts("")
    IO.puts("Next:")
    IO.puts("  1. Paste blake3 into World.DemoSceneVector @blake3")
    IO.puts("     and alchemy_protocol::demo_scene_vector::DEMO_SCENE_VECTOR_BLAKE3")
    IO.puts("  2. Paste YAW45_* lines into alchemy_shared_sim::demo (if changed)")
    IO.puts("  3. cargo test -p alchemy-protocol demo_scene_vector")
    IO.puts("  4. cargo test -p alchemy-shared-sim demo_scene_seal")

  {:error, :cargo_not_found} ->
    IO.puts("yaw_bits: skipped (cargo not on PATH)")
    IO.puts("")
    IO.puts("Next:")
    IO.puts("  1. Paste blake3 into @blake3 / DEMO_SCENE_VECTOR_BLAKE3")
    IO.puts("  2. From protocol/: cargo run -q --bin demo-scene-vector-yaw-bits -- #{out}")
    IO.puts("  3. Paste YAW45_* into alchemy_shared_sim::demo; run vector tests")

  {yaw_out, code} ->
    IO.puts("yaw_bits: cargo failed exit=#{code}")
    IO.puts(yaw_out)
    IO.puts("")
    IO.puts("Next: fix cargo/protocol build, then re-run this script (blake3 JSON is already written)")
end
