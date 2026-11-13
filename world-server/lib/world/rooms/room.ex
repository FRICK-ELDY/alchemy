defmodule World.Rooms.Room do
  @moduledoc """
  Persistent room ledger row (x0028).

  Guarantees: `room_id` + `owner_sub` + DemoScene `scene` + `visibility` + `wire_cap` (x004e) +
  `room_key` (x0069) + `room_key_epoch` (x007e) + host session (`active_host_sub` / `state` /
  `host_lease_expires_at` · x0080) + `editor_subs` (x009d) + `session_overlay` /
  `eligible_host_subs` (x009e) + handover candidate (x00a1) survive restart.
  Non-guarantees: Presence / physics pose; UGC edit history; previous-epoch key durability
  (grace is memory-only via `RoomKeyGrace`); seamless sim handoff of poses; Dash Permissions UI;
  overlay→canonical Save/merge.
  """

  use Ash.Resource,
    otp_app: :world,
    domain: World.Domain,
    data_layer: AshSqlite.DataLayer

  sqlite do
    table "rooms"
    repo World.Repo
  end

  attributes do
    attribute :room_id, :string do
      primary_key? true
      allow_nil? false
      public? true
    end

    attribute :owner_sub, :string do
      allow_nil? false
      public? true
    end

    # DemoScene map: %{"revision" => …, "entries" => […], "locomotion" => %{…}}
    attribute :scene, :map do
      allow_nil? false
      public? true
    end

    attribute :visibility, :string do
      allow_nil? false
      public? true
      default "public"

      constraints match: ~r/^(lan|private|friends|friends_plus|followers|followers_plus|public)$/
    end

    # x004e: Zenoh key segment secret (base64url-ish [A-Za-z0-9_-]+).
    attribute :wire_cap, :string do
      allow_nil? false
      public? true
    end

    # x0069: 32-byte Room AEAD key as 64 lowercase hex (API: URL-safe base64).
    attribute :room_key, :string do
      allow_nil? false
      public? true
    end

    # x007e: monotonic Room key generation; bumps on visibility narrow / owner rotate.
    attribute :room_key_epoch, :integer do
      allow_nil? false
      public? true
      default 0
    end

    # x007e: last successful key rotate (periodic / narrow / owner).
    attribute :room_key_rotated_at, :utc_datetime do
      allow_nil? true
      public? true
    end

    # x003a: host generation. 0 = none yet; bumps when attested host_pub changes.
    attribute :host_generation, :integer do
      allow_nil? false
      public? true
      default 0
    end

    # base64url (no pad) of last attested 32-byte host_pub; nil until first host_keys.
    attribute :attested_host_pub, :string do
      allow_nil? true
      public? true
    end

    # x0080: current compute host (nullable). Distinct from persistent `owner_sub`.
    attribute :active_host_sub, :string do
      allow_nil? true
      public? true
    end

    # x0080: session projection — Public catalog is `active` only.
    attribute :state, :string do
      allow_nil? false
      public? true
      default "dormant"

      constraints match: ~r/^(active|dormant)$/
    end

    # x0080: host heartbeat deadline while `state = active`.
    attribute :host_lease_expires_at, :utc_datetime do
      allow_nil? true
      public? true
    end

    # x009d: JSON array of subs (Builders) allowed to mutate canonical scene with owner.
    # Stored as text for SQLite / raw SQL; decoded in `Rooms.ensure_scene_writer/2`.
    attribute :editor_subs, :string do
      allow_nil? false
      public? true
      default "[]"
    end

    # x009e: temporary session entries (dormant clears). Map or JSON text from SQLite.
    attribute :session_overlay, :map do
      allow_nil? false
      public? true
      default %{"revision" => 0, "entries" => []}
    end

    # x009e: takeover allow-list (owner always allowed). Empty = owner-only.
    attribute :eligible_host_subs, :string do
      allow_nil? false
      public? true
      default "[]"
    end

    # x00a1: nominated host candidate (dormant until accept).
    attribute :handover_candidate_sub, :string do
      allow_nil? true
      public? true
    end

    attribute :handover_candidate_expires_at, :utc_datetime do
      allow_nil? true
      public? true
    end

    create_timestamp :inserted_at
    update_timestamp :updated_at
  end

  # Uniqueness is the primary key on `room_id` (no redundant Ash identity).

  actions do
    defaults [:read]

    create :create do
      primary? true

      accept [
        :room_id,
        :owner_sub,
        :scene,
        :visibility,
        :wire_cap,
        :room_key,
        :room_key_epoch,
        :room_key_rotated_at,
        :editor_subs,
        :session_overlay,
        :eligible_host_subs
      ]
    end

    update :set_visibility do
      # May also rotate room_key + room_key_epoch when visibility narrows (x007e).
      accept [:visibility, :room_key, :room_key_epoch, :room_key_rotated_at]
    end

    update :rotate_room_key do
      accept [:room_key, :room_key_epoch, :room_key_rotated_at]
    end

    # Test / admin corrupt-scene injection only. Production append/delete must use
    # Rooms.append_scene_entry/3 · delete_scene_entry/3 (x0082 revision CAS SQL).
    # Do not call set_scene from host HTTP paths — it bypasses expected_revision.
    update :set_scene do
      accept [:scene]
    end

    # host_generation / attested_host_pub: only via Rooms.claim_host_generation CAS SQL
    # (x003a). No Ash update action — avoids a non-CAS write path.

    read :get_by_room_id do
      get? true
      argument :room_id, :string, allow_nil?: false
      filter expr(room_id == ^arg(:room_id))
    end
  end

  code_interface do
    define :create, action: :create
    define :get_by_room_id, action: :get_by_room_id, args: [:room_id]
    define :set_scene, action: :set_scene
    define :set_visibility, action: :set_visibility
    define :rotate_room_key, action: :rotate_room_key
  end
end
