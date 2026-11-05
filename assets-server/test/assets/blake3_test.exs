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
end
