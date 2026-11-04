defmodule Assets.PrefabPayloadTest do
  @moduledoc "Tests for x005e Prefab.Payload schema validation."
  use ExUnit.Case, async: true

  alias Assets.Prefab.Payload

  @blake3 String.duplicate("a", 64)
  @blake3b String.duplicate("b", 64)

  # ---------------------------------------------------------------------------
  # Valid payloads
  # ---------------------------------------------------------------------------

  test "minimal valid payload (empty children)" do
    json = ~s({"schema_version":1,"children":[]})
    assert {:ok, %Payload{schema_version: 1, children: []}} = Payload.from_json(json)
  end

  test "child without trs" do
    json = Jason.encode!(%{schema_version: 1, children: [%{local_id: "m0", blake3: @blake3}]})
    assert {:ok, %Payload{children: [child]}} = Payload.from_json(json)
    assert child.local_id == "m0"
    assert child.blake3 == @blake3
    assert is_nil(child.trs)
  end

  test "child with valid trs" do
    json =
      Jason.encode!(%{
        schema_version: 1,
        children: [
          %{
            local_id: "c0",
            blake3: @blake3,
            trs: %{
              translation: [1.0, 2.0, 3.0],
              rotation: [0.0, 0.0, 0.0, 1.0],
              scale: [1.0, 1.0, 1.0]
            }
          }
        ]
      })

    assert {:ok, %Payload{children: [child]}} = Payload.from_json(json)
    assert child.trs.translation == [1.0, 2.0, 3.0]
    assert child.trs.rotation == [0.0, 0.0, 0.0, 1.0]
    assert child.trs.scale == [1.0, 1.0, 1.0]
  end

  test "integer TRS values are coerced to floats" do
    json =
      Jason.encode!(%{
        schema_version: 1,
        children: [
          %{
            local_id: "c0",
            blake3: @blake3,
            trs: %{translation: [1, 2, 3], rotation: [0, 0, 0, 1], scale: [1, 1, 1]}
          }
        ]
      })

    assert {:ok, %Payload{children: [child]}} = Payload.from_json(json)
    assert Enum.all?(child.trs.translation, &is_float/1)
  end

  test "multiple children" do
    json =
      Jason.encode!(%{
        schema_version: 1,
        children: [
          %{local_id: "c0", blake3: @blake3},
          %{local_id: "c1", blake3: @blake3b}
        ]
      })

    assert {:ok, %Payload{children: children}} = Payload.from_json(json)
    assert length(children) == 2
  end

  # ---------------------------------------------------------------------------
  # Round-trip (to_json / from_json)
  # ---------------------------------------------------------------------------

  test "to_json / from_json round-trip preserves logical structure" do
    {:ok, payload} =
      Payload.new(1, [
        %{local_id: "m0", blake3: @blake3, trs: nil},
        %{
          local_id: "m1",
          blake3: @blake3b,
          trs: %{
            translation: [0.5, 1.0, -1.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [2.0, 2.0, 2.0]
          }
        }
      ])

    assert {:ok, json} = Payload.to_json(payload)
    assert {:ok, decoded} = Payload.from_json(json)

    assert decoded.schema_version == payload.schema_version
    assert length(decoded.children) == 2

    [c0, c1] = decoded.children
    assert c0.local_id == "m0"
    assert c0.blake3 == @blake3
    assert is_nil(c0.trs)
    assert c1.local_id == "m1"
    assert c1.trs.scale == [2.0, 2.0, 2.0]
  end

  # ---------------------------------------------------------------------------
  # Invalid payloads
  # ---------------------------------------------------------------------------

  test "rejects unknown schema_version" do
    json = ~s({"schema_version":99,"children":[]})
    assert {:error, {:unsupported_schema_version, 99}} = Payload.from_json(json)
  end

  test "rejects missing schema_version" do
    json = ~s({"children":[]})
    assert {:error, :invalid_payload_shape} = Payload.from_json(json)
  end

  test "rejects invalid JSON" do
    assert {:error, _} = Payload.from_json("not json")
  end

  test "rejects child with missing blake3" do
    json = Jason.encode!(%{schema_version: 1, children: [%{local_id: "c0"}]})
    assert {:error, {:invalid_child, 0, :invalid_child_shape}} = Payload.from_json(json)
  end

  test "rejects child with wrong-length blake3" do
    json = Jason.encode!(%{schema_version: 1, children: [%{local_id: "c0", blake3: "tooshort"}]})
    assert {:error, {:invalid_child, 0, :invalid_child_shape}} = Payload.from_json(json)
  end

  test "rejects child with empty local_id" do
    json = Jason.encode!(%{schema_version: 1, children: [%{local_id: "", blake3: @blake3}]})
    assert {:error, {:invalid_child, 0, :invalid_child_shape}} = Payload.from_json(json)
  end

  test "rejects non-finite TRS (Inf not possible in JSON but guarded at float level)" do
    # We test non-finite detection by injecting via validate/1 directly
    # (standard JSON cannot encode Inf/NaN, so we test the guard path)
    result =
      Payload.validate(%{
        "schema_version" => 1,
        "children" => [
          %{
            "local_id" => "x",
            "blake3" => @blake3,
            "trs" => %{
              "translation" => [1.0, 2.0, "not_a_number"],
              "rotation" => [0.0, 0.0, 0.0, 1.0],
              "scale" => [1.0, 1.0, 1.0]
            }
          }
        ]
      })

    # "not_a_number" passes the list-length guard but fails the finite_numeric? check
    assert {:error, {:invalid_child, 0, :non_finite_trs}} = result
  end

  test "rejects TRS with wrong vector length" do
    json =
      Jason.encode!(%{
        schema_version: 1,
        children: [
          %{
            local_id: "c",
            blake3: @blake3,
            trs: %{
              translation: [1.0, 2.0],
              rotation: [0.0, 0.0, 0.0, 1.0],
              scale: [1.0, 1.0, 1.0]
            }
          }
        ]
      })

    assert {:error, {:invalid_child, 0, :invalid_trs_shape}} = Payload.from_json(json)
  end
end
