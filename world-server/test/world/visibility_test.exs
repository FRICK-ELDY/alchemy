defmodule World.VisibilityTest do
  use ExUnit.Case, async: true

  alias World.Visibility

  test "rank increases toward public" do
    assert Visibility.rank("lan") < Visibility.rank("private")
    assert Visibility.rank("private") < Visibility.rank("friends")
    assert Visibility.rank("friends") < Visibility.rank("public")
  end

  test "narrower?/2 detects closed moves (x007e)" do
    assert Visibility.narrower?("friends", "public")
    assert Visibility.narrower?("private", "friends")
    refute Visibility.narrower?("public", "friends")
    refute Visibility.narrower?("friends", "friends")
    refute Visibility.narrower?("bogus", "public")
  end
end
