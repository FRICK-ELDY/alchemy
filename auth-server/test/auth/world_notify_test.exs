defmodule Auth.WorldNotifyTest do
  use ExUnit.Case, async: false

  alias Auth.WorldNotify

  setup do
    previous_mode = Application.get_env(:auth, :world_notify_mode)
    previous_stub = Application.get_env(:auth, :world_notify_stub)
    Application.put_env(:auth, :world_notify_mode, :stub)

    on_exit(fn ->
      restore(:world_notify_mode, previous_mode)
      restore(:world_notify_stub, previous_stub)
    end)

    :ok
  end

  defp restore(key, nil), do: Application.delete_env(:auth, key)
  defp restore(key, value), do: Application.put_env(:auth, key, value)

  test "stub receives relation_changed (x00a0)" do
    parent = self()
    Application.put_env(:auth, :world_notify_stub, parent)

    assert :ok = WorldNotify.relation_changed("aaa", "bbb", "unfriend")
    assert_receive {:relation_changed, "aaa", "bbb", "unfriend"}, 500
  end
end
