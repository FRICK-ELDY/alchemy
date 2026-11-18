defmodule Auth.SocialTest do
  use Auth.DataCase, async: true

  import Auth.AccountsFixtures

  alias Auth.{Access, Social}
  alias Auth.WorldNotify.Outbox

  setup do
    alice = user_fixture(%{username: "alice_soc"})
    bob = user_fixture(%{username: "bob_soc"})
    carol = user_fixture(%{username: "carol_soc"})
    %{alice: alice, bob: bob, carol: carol}
  end

  test "follow and unfollow", %{alice: alice, bob: bob} do
    assert {:ok, _} = Social.follow(alice.id, bob.id)
    assert Social.following?(alice.id, bob.id)
    assert {:error, :already_following} = Social.follow(alice.id, bob.id)
    assert :ok = Social.unfollow(alice.id, bob.id)
    refute Social.following?(alice.id, bob.id)
  end

  test "plain unfollow enqueues relation outbox (x00d7)", %{alice: alice, bob: bob} do
    parent = self()
    previous_stub = Application.get_env(:auth, :world_notify_stub)
    Application.put_env(:auth, :world_notify_stub, parent)

    on_exit(fn ->
      if previous_stub,
        do: Application.put_env(:auth, :world_notify_stub, previous_stub),
        else: Application.delete_env(:auth, :world_notify_stub)
    end)

    assert {:ok, _} = Social.follow(alice.id, bob.id)
    refute Social.friends?(alice.id, bob.id)

    assert :ok = Social.unfollow(alice.id, bob.id)

    assert_receive {:relation_changed, event_id, sub_a, sub_b, "unfollow", owners}, 500
    assert sub_a == alice.id
    assert sub_b == bob.id
    assert bob.id in owners
    assert alice.id in owners

    row = Outbox.get(event_id)
    assert row.status == "delivered"
    assert row.reason == "unfollow"
  end

  test "affected_owner_subs includes FoF friends (x00d7)", %{
    alice: alice,
    bob: bob,
    carol: carol
  } do
    for {a, b} <- [{alice, bob}, {bob, carol}] do
      assert {:ok, _} = Social.follow(a.id, b.id)
      assert {:ok, _} = Social.follow(b.id, a.id)
      assert {:ok, _} = Social.request_friend(a.id, b.id)
      assert {:ok, _} = Social.accept_friend(b.id, a.id)
    end

    owners = Social.affected_owner_subs(bob.id, carol.id)
    assert alice.id in owners
    assert bob.id in owners
    assert carol.id in owners
  end

  test "cannot follow self", %{alice: alice} do
    assert {:error, :self} = Social.follow(alice.id, alice.id)
  end

  test "friend request requires mutual follow", %{alice: alice, bob: bob} do
    assert {:error, :not_mutual} = Social.request_friend(alice.id, bob.id)

    assert {:ok, _} = Social.follow(alice.id, bob.id)
    assert {:error, :not_mutual} = Social.request_friend(alice.id, bob.id)

    assert {:ok, _} = Social.follow(bob.id, alice.id)
    assert {:ok, row} = Social.request_friend(alice.id, bob.id)
    assert row.state == :pending
  end

  test "accept makes friends; unfollow clears friendship", %{alice: alice, bob: bob} do
    assert {:ok, _} = Social.follow(alice.id, bob.id)
    assert {:ok, _} = Social.follow(bob.id, alice.id)
    assert {:ok, _} = Social.request_friend(alice.id, bob.id)
    assert {:ok, _} = Social.accept_friend(bob.id, alice.id)
    assert Social.friends?(alice.id, bob.id)

    assert :ok = Social.unfollow(alice.id, bob.id)
    refute Social.mutual?(alice.id, bob.id)
    refute Social.friends?(alice.id, bob.id)
  end

  test "unfollow clears pending friend request", %{alice: alice, bob: bob} do
    assert {:ok, _} = Social.follow(alice.id, bob.id)
    assert {:ok, _} = Social.follow(bob.id, alice.id)
    assert {:ok, _} = Social.request_friend(alice.id, bob.id)

    assert :ok = Social.unfollow(bob.id, alice.id)
    refute Social.friends?(alice.id, bob.id)
    assert {:error, :not_found} = Social.accept_friend(bob.id, alice.id)
  end

  test "friends_plus is friends-of-friends", %{alice: alice, bob: bob, carol: carol} do
    # alice-bob friends, bob-carol friends → carol in alice friends_plus
    for {a, b} <- [{alice, bob}, {bob, carol}] do
      assert {:ok, _} = Social.follow(a.id, b.id)
      assert {:ok, _} = Social.follow(b.id, a.id)
      assert {:ok, _} = Social.request_friend(a.id, b.id)
      assert {:ok, _} = Social.accept_friend(b.id, a.id)
    end

    assert Social.friends_plus?(alice.id, carol.id)
    refute Social.friends?(alice.id, carol.id)
    assert Access.allowed?(alice.id, carol.id, :friends_plus)
    refute Access.allowed?(alice.id, carol.id, :friends)
  end

  test "followers and followers_plus", %{alice: alice, bob: bob, carol: carol} do
    # bob follows alice; carol follows bob
    assert {:ok, _} = Social.follow(bob.id, alice.id)
    assert {:ok, _} = Social.follow(carol.id, bob.id)

    assert Social.follower?(alice.id, bob.id)
    refute Social.follower?(alice.id, carol.id)
    assert Social.followers_plus?(alice.id, carol.id)
    assert Access.allowed?(alice.id, bob.id, :followers)
    refute Access.allowed?(alice.id, carol.id, :followers)
    assert Access.allowed?(alice.id, carol.id, :followers_plus)
  end

  test "access private and public", %{alice: alice, bob: bob} do
    assert Access.allowed?(alice.id, alice.id, :private)
    refute Access.allowed?(alice.id, bob.id, :private)
    assert Access.allowed?(alice.id, bob.id, :public)
    assert Access.allowed?(alice.id, bob.id, :lan)
  end
end
