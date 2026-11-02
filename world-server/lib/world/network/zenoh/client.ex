defmodule World.Network.Zenoh.Client do
  @moduledoc """
  Unified Zenoh entry point for world-server business code.

  Guarantees: all publish/subscribe go through the configured
  `World.Network.Zenoh.Adapter` (`:world, :zenoh_adapter`); subscribed
  samples arrive as `World.Network.Zenoh.Adapter.sample_message/0`
  (`{:zenoh_sample, key_expr, payload}`).
  Non-guarantees: which adapter is live; key expression layout; payload
  crypto; session survival / automatic re-subscribe after disconnect;
  a sample already in flight at `unsubscribe/1` time may still arrive once.
  """

  alias World.Network.Zenoh.Adapter

  @type sample_message :: Adapter.sample_message()

  @spec publish(String.t(), binary()) :: :ok | {:error, term()}
  def publish(key_expr, payload)
      when is_binary(key_expr) and is_binary(payload) do
    adapter().publish(key_expr, payload)
  end

  @spec subscribe(String.t(), pid()) :: {:ok, term()} | {:error, term()}
  def subscribe(key_expr, subscriber_pid \\ self())
      when is_binary(key_expr) and is_pid(subscriber_pid) do
    adapter().subscribe(key_expr, subscriber_pid)
  end

  @spec unsubscribe(term()) :: :ok | {:error, term()}
  def unsubscribe(sub_id) do
    adapter().unsubscribe(sub_id)
  end

  defp adapter do
    Application.get_env(:world, :zenoh_adapter, World.Network.Zenoh.ZenohexAdapter)
  end
end
