defmodule Assets.Blake3Test do
  use ExUnit.Case, async: false

  # Official BLAKE3 unkeyed test vectors (empty / "abc").
  @empty "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
  @abc "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"

  test "hash_hex empty and abc via permanent Port" do
    assert Assets.Blake3.hash_hex("") == @empty
    assert Assets.Blake3.hash_hex("abc") == @abc
  end

  test "hash_hex reuses worker for successive calls" do
    assert Assets.Blake3.hash_hex("abc") == @abc
    assert Assets.Blake3.hash_hex("") == @empty
    assert Assets.Blake3.hash_hex("abc") == @abc
  end

  test "hash_hex reopens Port after forced close (fail-closed then recover)" do
    %{port: old} = :sys.get_state(Assets.Blake3.Worker)
    assert is_port(old)
    Port.close(old)

    new_port = await_replaced_port(Assets.Blake3.Worker, old)
    assert is_port(new_port)
    assert new_port != old

    assert Assets.Blake3.hash_hex("abc") == @abc
    assert Assets.Blake3.hash_hex("") == @empty
  end

  defp await_replaced_port(name, old, attempts \\ 50) do
    Enum.find_value(1..attempts, fn _ ->
      Process.sleep(20)

      case :sys.get_state(name) do
        %{port: port} when is_port(port) and port != old -> port
        _ -> nil
      end
    end) ||
      flunk("Port was not replaced after close (old=#{inspect(old)})")
  end
end
