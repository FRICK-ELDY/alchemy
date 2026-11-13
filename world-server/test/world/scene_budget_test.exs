defmodule World.SceneBudgetTest do
  use ExUnit.Case, async: true

  alias World.SceneBudget

  test "encode_checked accepts empty scene under limits" do
    assert {:ok, json} = SceneBudget.encode_checked(%{"revision" => 0, "entries" => []})
    assert is_binary(json)
  end

  test "encode_checked rejects too many entries (x00a2)" do
    entries =
      for i <- 1..(SceneBudget.max_entries() + 1) do
        %{"entity_id" => i, "blake3" => String.duplicate("ab", 32)}
      end

    assert {:error, :scene_entries_exceeded} =
             SceneBudget.encode_checked(%{"revision" => 1, "entries" => entries})
  end

  test "encode_checked rejects oversized JSON bytes (x00a2)" do
    # One entry with a huge padding field to blow past 1 MiB without exceeding entry count.
    padding = String.duplicate("x", SceneBudget.max_json_bytes())

    assert {:error, :scene_bytes_exceeded} =
             SceneBudget.encode_checked(%{
               "revision" => 1,
               "entries" => [%{"entity_id" => 1, "blake3" => String.duplicate("cd", 32)}],
               "pad" => padding
             })
  end

  test "ensure_composed rejects canon+overlay over entry cap" do
    half = div(SceneBudget.max_entries(), 2) + 1

    canon = %{
      "revision" => 1,
      "entries" =>
        for i <- 1..half do
          %{"entity_id" => i, "blake3" => String.duplicate("ab", 32)}
        end
    }

    overlay = %{
      "revision" => 1,
      "entries" =>
        for i <- 1..half do
          %{"entity_id" => 10_000 + i, "blake3" => String.duplicate("cd", 32)}
        end
    }

    assert {:error, :scene_entries_exceeded} = SceneBudget.ensure_composed(canon, overlay)
  end
end
