defmodule World.Network.Zenoh.Adapter do
  @moduledoc """
  Behaviour for Zenoh publish/subscribe backends.

  Guarantees: a swappable transport behind `World.Network.Zenoh.Client`;
  every successful subscription delivers samples to `subscriber_pid` as
  `sample_message/0` only (see `sample_message/2`).
  Non-guarantees: topic schema; AEAD; mTLS; delivery/ordering across adapters;
  automatic re-subscribe after session loss; a sample already in flight at
  unsubscribe time may still arrive once.
  """

  @type key_expr :: String.t()
  @type sub_id :: term()
  @type sample_message :: {:zenoh_sample, key_expr(), binary()}

  @callback publish(key_expr(), payload :: binary()) :: :ok | {:error, term()}
  @callback subscribe(key_expr(), subscriber_pid :: pid()) :: {:ok, sub_id()} | {:error, term()}
  @callback unsubscribe(sub_id()) :: :ok | {:error, term()}

  @doc """
  Canonical mailbox message for subscribed samples.

  Adapters must send this shape (never backend-native structs such as
  `%Zenohex.Sample{}`) so business code stays adapter-agnostic.
  """
  @spec sample_message(key_expr(), binary()) :: sample_message()
  def sample_message(key_expr, payload)
      when is_binary(key_expr) and is_binary(payload) do
    {:zenoh_sample, key_expr, payload}
  end

  @doc """
  Sends `sample_message/2` to `subscriber_pid`.
  """
  @spec deliver_sample(pid(), key_expr(), binary()) :: sample_message()
  def deliver_sample(subscriber_pid, key_expr, payload)
      when is_pid(subscriber_pid) and is_binary(key_expr) and is_binary(payload) do
    message = sample_message(key_expr, payload)
    send(subscriber_pid, message)
    message
  end
end
