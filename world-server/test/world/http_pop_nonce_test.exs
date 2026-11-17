defmodule World.HttpPop.NonceTest do
  use ExUnit.Case, async: false

  alias World.HttpPop
  alias World.HttpPop.Nonce

  setup do
    Nonce.ensure_table!()
    :ok
  end

  test "claim accepts once and rejects reuse" do
    jti = "jti-reuse-#{System.unique_integer([:positive])}"
    nonce = "n1"
    ts = System.system_time(:second)

    assert :ok = Nonce.claim(jti, nonce, ts)
    assert {:error, :pop_nonce_reused} = Nonce.claim(jti, nonce, ts)
  end

  test "claim TTL is pop_ts + skew (covers future-skewed window)" do
    jti = "jti-future-#{System.unique_integer([:positive])}"
    nonce = "n-future"
    now = System.system_time(:second)
    skew = HttpPop.skew_secs()
    # Host clock ahead: still within ±skew.
    ts = now + skew - 5

    assert :ok = Nonce.claim(jti, nonce, ts)
    assert {:ok, exp} = Nonce.consume_exp(jti, nonce)
    assert exp == ts + skew
    # At now+skew+1 the old claim-relative TTL (now+skew) would have pruned, but
    # skew still accepts this ts. Entry must outlive that point.
    assert exp > now + skew + 1
    assert {:error, :pop_nonce_reused} = Nonce.claim(jti, nonce, ts)
  end

  test "reclaim after expiry prune" do
    jti = "jti-expire-#{System.unique_integer([:positive])}"
    nonce = "n-expire"
    ts = System.system_time(:second)

    assert :ok = Nonce.claim(jti, nonce, ts)
    assert :ok = Nonce.put_exp_for_test!(jti, nonce, ts - 1)
    assert :ok = Nonce.force_prune_expired!()
    assert :error = Nonce.consume_exp(jti, nonce)
    assert :ok = Nonce.claim(jti, nonce, ts)
  end

  test "invalid nonce shapes" do
    ts = System.system_time(:second)
    assert {:error, :invalid_pop_nonce} = Nonce.claim("", "n", ts)
    assert {:error, :invalid_pop_nonce} = Nonce.claim("jti", "", ts)
    assert {:error, :invalid_pop_nonce} = Nonce.claim("jti", String.duplicate("a", 129), ts)
    assert {:error, :invalid_pop_nonce} = Nonce.claim("jti", "n", -1)
  end

  test "force_prune keeps unexpired entries" do
    jti = "jti-keep-#{System.unique_integer([:positive])}"
    nonce = "n-keep"
    ts = System.system_time(:second)

    assert :ok = Nonce.claim(jti, nonce, ts)
    assert :ok = Nonce.force_prune_expired!()
    assert {:ok, _} = Nonce.consume_exp(jti, nonce)
    assert {:error, :pop_nonce_reused} = Nonce.claim(jti, nonce, ts)
  end
end
