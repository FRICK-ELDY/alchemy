defmodule World.MtlsIssuer do
  @moduledoc """
  x0066: Sign short-lived mTLS client certs from a CSR using openssl CLI (no Hex x509 dep).

  Guarantees: CSR Subject is ignored; CN is forced from server-authorized role
  (`alchemy-host` | `alchemy-guest`). Non-guarantees: sub-day openssl NotAfter
  (tooling uses `-days 1`); soft API TTL is separate (`expires_at`).
  """

  require Logger

  @cn_host "alchemy-host"
  @cn_guest "alchemy-guest"
  # Soft refresh SLA returned to clients (openssl NotAfter floor is 1 day).
  @default_ttl_seconds 6 * 3600
  @openssl_days "1"

  @type role :: String.t()

  @doc """
  True when CA cert + key PEM (or readable path files) are configured.
  """
  def configured? do
    match?({:ok, _, _}, load_ca())
  end

  @doc """
  Sign `csr_pem` for an authorized `role` (`"host"` | `"guest"`).

  Returns PEM cert + CA PEM + `expires_at` (UTC soft TTL). Does not return the
  client private key (client already holds it).
  """
  @spec issue(String.t(), role(), keyword()) ::
          {:ok, %{client_cert_pem: String.t(), ca_cert_pem: String.t(), expires_at: DateTime.t()}}
          | {:error, atom()}
  def issue(csr_pem, role, opts \\ [])

  def issue(csr_pem, role, opts)
      when is_binary(csr_pem) and csr_pem != "" and role in ["host", "guest"] do
    ttl = Keyword.get(opts, :ttl_seconds, @default_ttl_seconds)
    cn = if(role == "host", do: @cn_host, else: @cn_guest)
    work = work_dir()

    with :ok <- File.mkdir_p(work),
         {:ok, ca_cert_path, ca_key_path, ca_pem} <- materialize_ca(work),
         {:ok, paths} <- write_inputs(work, csr_pem),
         :ok <- run_sign(paths, ca_cert_path, ca_key_path, cn),
         {:ok, cert_pem} <- File.read(paths.cert) do
      cleanup(paths)
      File.rm(ca_cert_path)
      File.rm(ca_key_path)
      now = DateTime.utc_now() |> DateTime.truncate(:second)
      expires_at = DateTime.add(now, ttl, :second)

      {:ok,
       %{
         client_cert_pem: cert_pem,
         ca_cert_pem: ca_pem,
         expires_at: expires_at
       }}
    else
      {:error, reason} = err when is_atom(reason) ->
        err

      {:error, reason} ->
        Logger.warning("mtls_issuer failed: #{inspect(reason)}")
        {:error, :sign_failed}
    end
  end

  def issue(_, _, _), do: {:error, :invalid_args}

  defp work_dir do
    Application.get_env(:world, :mtls_work_dir) ||
      Path.join(System.tmp_dir!(), "alchemy-world-mtls")
  end

  defp openssl_bin do
    System.get_env("OPENSSL_BIN") ||
      Application.get_env(:world, :openssl_bin) ||
      detect_openssl()
  end

  defp detect_openssl do
    [
      "openssl",
      "C:/Program Files/Git/usr/bin/openssl.exe",
      "C:/Program Files/OpenSSL-Win64/bin/openssl.exe"
    ]
    |> Enum.find(&openssl_ok?/1) || "openssl"
  end

  defp openssl_ok?(bin) do
    match?({_, 0}, System.cmd(bin, ["version"], stderr_to_stdout: true))
  rescue
    _ -> false
  end

  defp load_ca do
    cert = Application.get_env(:world, :mtls_ca_cert_pem)
    key = Application.get_env(:world, :mtls_ca_key_pem)

    cert_path = Application.get_env(:world, :mtls_ca_cert_path)
    key_path = Application.get_env(:world, :mtls_ca_key_path)

    cond do
      is_binary(cert) and cert != "" and is_binary(key) and key != "" ->
        {:ok, cert, key}

      is_binary(cert_path) and cert_path != "" and is_binary(key_path) and key_path != "" ->
        with {:ok, c} <- File.read(cert_path),
             {:ok, k} <- File.read(key_path) do
          {:ok, c, k}
        else
          _ -> {:error, :ca_unreadable}
        end

      true ->
        {:error, :ca_not_configured}
    end
  end

  defp materialize_ca(work) do
    with {:ok, cert_pem, key_pem} <- load_ca() do
      stamp = System.system_time(:nanosecond)
      cert_path = Path.join(work, "ca-#{stamp}.cert.pem")
      key_path = Path.join(work, "ca-#{stamp}.key.pem")

      with :ok <- File.write(cert_path, cert_pem),
           :ok <- File.write(key_path, key_pem),
           :ok <- chmod_secret(key_path) do
        {:ok, cert_path, key_path, cert_pem}
      end
    end
  end

  defp write_inputs(work, csr_pem) do
    stamp = System.system_time(:nanosecond)
    csr = Path.join(work, "req-#{stamp}.csr.pem")
    cert = Path.join(work, "req-#{stamp}.cert.pem")
    ext = Path.join(work, "req-#{stamp}.ext")

    ext_body =
      "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n"

    with :ok <- File.write(csr, csr_pem),
         :ok <- File.write(ext, ext_body) do
      {:ok, %{csr: csr, cert: cert, ext: ext, stamp: stamp}}
    end
  end

  defp run_sign(paths, ca_cert, ca_key, cn) do
    serial = Integer.to_string(System.system_time(:nanosecond), 16)
    # -subj replaces CSR subject (do not trust client-supplied CN).
    args = [
      "x509",
      "-req",
      "-in",
      paths.csr,
      "-CA",
      ca_cert,
      "-CAkey",
      ca_key,
      "-set_serial",
      "0x#{serial}",
      "-out",
      paths.cert,
      "-days",
      @openssl_days,
      "-subj",
      "/CN=#{cn}",
      "-extfile",
      paths.ext
    ]

    case System.cmd(openssl_bin(), args, stderr_to_stdout: true) do
      {_, 0} ->
        :ok

      {out, code} ->
        Logger.warning("openssl x509 failed code=#{code}: #{String.slice(out, 0, 500)}")
        {:error, :openssl_failed}
    end
  end

  defp cleanup(paths) do
    Enum.each([paths.csr, paths.cert, paths.ext], &File.rm/1)
    :ok
  end

  defp chmod_secret(path) do
    case File.chmod(path, 0o600) do
      :ok -> :ok
      {:error, _} -> :ok
    end
  end
end
