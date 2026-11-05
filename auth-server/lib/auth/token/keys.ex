defmodule Auth.Token.Keys do
  @moduledoc """
  Loads or generates the RS256 signing key and exposes JWKS for verification.

  The active key (`jwt_private_key_path`) signs new tokens. Additional keys
  (`jwt_verification_key_paths`) are published in JWKS for verification during
  key rotation grace periods.

  Hot-path reads (`signer/0`, `jwks/0`, `signer_for_kid/1`) use `:persistent_term`
  so login/refresh do not serialize through this GenServer (x0073). Keys are
  loaded **synchronously in `init/1`** so `start_link` does not return until the
  term is populated (same “ready before Endpoint” ordering as the old
  `GenServer.call` gate). Unready reads fail closed (raise) — never an empty JWKS.
  """

  use GenServer

  @pt_key {__MODULE__, :key_state}

  # Explicit shutdown so supervisor stop invokes `terminate/2` (PT erase), not brutal_kill.
  def child_spec(opts) do
    %{
      id: __MODULE__,
      start: {__MODULE__, :start_link, [opts]},
      type: :worker,
      restart: :permanent,
      shutdown: 5_000
    }
  end

  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: __MODULE__)
  end

  @doc """
  Confirm this process finished `init/1` (keys already in `:persistent_term`).

  With sync init, registration implies readiness; this call is for tests that
  restart the child under a new Application env and need a barrier after
  `Supervisor.restart_child/2`.
  """
  @spec await_ready(timeout()) :: :ok
  def await_ready(timeout \\ 5_000), do: GenServer.call(__MODULE__, :await_ready, timeout)

  @spec signer() :: Joken.Signer.t()
  def signer, do: key_state!().active_signer

  @spec jwks() :: map()
  def jwks, do: key_state!().jwks

  @spec signer_for_kid(String.t()) :: {:ok, Joken.Signer.t()} | {:error, :unknown_kid}
  def signer_for_kid(kid) when is_binary(kid) do
    case Map.fetch(key_state!().signers_by_kid, kid) do
      {:ok, signer} -> {:ok, signer}
      :error -> {:error, :unknown_kid}
    end
  end

  @impl true
  def init(_opts) do
    state = load_key_state!()
    :persistent_term.put(@pt_key, state)
    {:ok, :ready}
  end

  @impl true
  def handle_call(:await_ready, _from, :ready), do: {:reply, :ok, :ready}

  @impl true
  def terminate(_reason, _state) do
    # Best-effort cleanup. Readers also fail closed when `whereis` is nil, so a
    # brutal_kill that skips this callback cannot serve empty JWKS as success.
    _ = :persistent_term.erase(@pt_key)
    :ok
  end

  defp key_state! do
    # Fail-closed if the process is down even when PT erase was skipped (brutal_kill).
    unless Process.whereis(__MODULE__) do
      raise "Auth.Token.Keys not ready"
    end

    case :persistent_term.get(@pt_key, :missing) do
      :missing ->
        raise "Auth.Token.Keys not ready"

      state ->
        state
    end
  end

  defp load_key_state! do
    active_path = Application.fetch_env!(:auth, :jwt_private_key_path)
    verification_paths = Application.get_env(:auth, :jwt_verification_key_paths, [])

    active_pem = load_or_generate_private_key!(active_path)
    verification_pems = Enum.map(verification_paths, &load_verification_key!/1)

    all_pems = [active_pem | verification_pems]
    key_entries = Enum.map(all_pems, &build_key_entry/1)
    ensure_unique_kids!(key_entries)

    active_entry = hd(key_entries)
    signers_by_kid = Map.new(key_entries, fn %{kid: kid, signer: signer} -> {kid, signer} end)

    %{
      active_signer: active_entry.signer,
      signers_by_kid: signers_by_kid,
      jwks: %{"keys" => Enum.map(key_entries, & &1.jwk)}
    }
  end

  defp ensure_unique_kids!(key_entries) do
    kids = Enum.map(key_entries, & &1.kid)
    unique_kids = Enum.uniq(kids)

    if length(kids) != length(unique_kids) do
      raise ArgumentError,
            "duplicate JWT key ids (kid) detected across jwt_private_key_path and jwt_verification_key_paths"
    end
  end

  defp build_key_entry(pem) do
    jwk = JOSE.JWK.from_pem(pem)
    public_jwk = JOSE.JWK.to_public(jwk)
    kid = thumbprint_kid(public_jwk)
    {_fields, jwk_map} = JOSE.JWK.to_map(public_jwk)

    %{
      kid: kid,
      signer: Joken.Signer.create("RS256", %{"pem" => pem}, %{"kid" => kid}),
      jwk:
        jwk_map
        |> Map.put("kid", kid)
        |> Map.put("use", "sig")
        |> Map.put("alg", "RS256")
    }
  end

  defp load_or_generate_private_key!(path) do
    if File.exists?(path) do
      File.read!(path)
    else
      if Application.get_env(:auth, :jwt_generate_key_on_startup, false) do
        generate_and_write_key_pair!(path)
      else
        raise ArgumentError,
              "JWT private key not found at #{path}. Key generation is disabled in this environment."
      end
    end
  end

  defp load_verification_key!(path) do
    unless File.exists?(path) do
      raise ArgumentError, "JWT verification key not found at #{path}"
    end

    File.read!(path)
  end

  defp generate_and_write_key_pair!(private_path) do
    jwk = JOSE.JWK.generate_key({:rsa, 2048})
    {_type, private_pem} = JOSE.JWK.to_pem(jwk)
    {_type, public_pem} = JOSE.JWK.to_pem(JOSE.JWK.to_public(jwk))

    public_path = public_key_path(private_path)

    File.mkdir_p!(Path.dirname(private_path))
    File.write!(private_path, private_pem)
    File.chmod!(private_path, 0o600)
    File.write!(public_path, public_pem)

    private_pem
  end

  defp public_key_path(private_path) do
    case String.replace(private_path, "private", "public") do
      ^private_path ->
        ext = Path.extname(private_path)
        Path.rootname(private_path) <> "_public" <> ext

      public_path ->
        public_path
    end
  end

  defp thumbprint_kid(jwk) do
    case JOSE.JWK.thumbprint(jwk) do
      {:kid, kid} -> kid
      kid when is_binary(kid) -> kid
    end
  end
end
