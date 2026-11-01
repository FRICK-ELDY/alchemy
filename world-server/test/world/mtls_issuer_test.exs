defmodule World.MtlsIssuerTest do
  use ExUnit.Case, async: false

  alias World.MtlsIssuer

  @moduletag :tmp_dir

  setup %{tmp_dir: tmp} do
    openssl = openssl_bin()
    ca_dir = Path.join(tmp, "ca")
    File.mkdir_p!(ca_dir)
    ca_key = Path.join(ca_dir, "ca-key.pem")
    ca_cert = Path.join(ca_dir, "ca-cert.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-x509",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          ca_key,
          "-out",
          ca_cert,
          "-days",
          "1",
          "-subj",
          "/CN=alchemy-test-client-ca"
        ],
        stderr_to_stdout: true
      )

    Application.put_env(:world, :mtls_ca_cert_path, ca_cert)
    Application.put_env(:world, :mtls_ca_key_path, ca_key)
    Application.put_env(:world, :mtls_work_dir, Path.join(tmp, "work"))

    on_exit(fn ->
      Application.delete_env(:world, :mtls_ca_cert_path)
      Application.delete_env(:world, :mtls_ca_key_path)
      Application.delete_env(:world, :mtls_ca_cert_pem)
      Application.delete_env(:world, :mtls_ca_key_pem)
      Application.delete_env(:world, :mtls_work_dir)
    end)

    Application.put_env(:world, :openssl_bin, openssl)
    %{ca_dir: ca_dir, tmp: tmp, openssl: openssl}
  end

  test "forces CN from role and ignores CSR subject", %{tmp: tmp, openssl: openssl} do
    csr = make_csr(tmp, "/CN=alchemy-core", openssl)

    assert {:ok, %{client_cert_pem: cert, ca_cert_pem: ca, expires_at: exp}} =
             MtlsIssuer.issue(csr, "guest")

    assert is_binary(ca) and String.contains?(ca, "BEGIN CERTIFICATE")
    assert String.contains?(cert, "BEGIN CERTIFICATE")
    assert %DateTime{} = exp

    subject = cert_subject(tmp, cert, openssl)
    assert subject =~ "CN = alchemy-guest" or subject =~ "CN=alchemy-guest"
    refute subject =~ "alchemy-core"
  end

  test "host role yields alchemy-host CN", %{tmp: tmp, openssl: openssl} do
    csr = make_csr(tmp, "/CN=ignored-host-claim", openssl)

    assert {:ok, %{client_cert_pem: cert}} = MtlsIssuer.issue(csr, "host")

    subject = cert_subject(tmp, cert, openssl)
    assert subject =~ "CN = alchemy-host" or subject =~ "CN=alchemy-host"
  end

  test "configured? false without CA" do
    Application.delete_env(:world, :mtls_ca_cert_path)
    Application.delete_env(:world, :mtls_ca_key_path)
    refute MtlsIssuer.configured?()
  end

  test "sign failure removes temp CA copies when CA is inline PEM", %{tmp: tmp} do
    ca_cert = Application.get_env(:world, :mtls_ca_cert_path)
    ca_key = Application.get_env(:world, :mtls_ca_key_path)
    cert_pem = File.read!(ca_cert)
    key_pem = File.read!(ca_key)

    Application.delete_env(:world, :mtls_ca_cert_path)
    Application.delete_env(:world, :mtls_ca_key_path)
    Application.put_env(:world, :mtls_ca_cert_pem, cert_pem)
    Application.put_env(:world, :mtls_ca_key_pem, key_pem)

    work = Path.join(tmp, "work")
    File.mkdir_p!(work)

    assert {:error, :openssl_failed} = MtlsIssuer.issue("not-a-real-csr", "guest")

    leftover =
      File.ls!(work)
      |> Enum.filter(&(String.starts_with?(&1, "ca-") or String.starts_with?(&1, "req-")))

    assert leftover == []
  end

  defp openssl_bin do
    found =
      [
        System.get_env("OPENSSL_BIN"),
        "openssl",
        "C:/Program Files/Git/usr/bin/openssl.exe",
        "C:/Program Files/OpenSSL-Win64/bin/openssl.exe"
      ]
      |> Enum.reject(&is_nil/1)
      |> Enum.find(&openssl_ok?/1)

    found || flunk("openssl not found (set OPENSSL_BIN)")
  end

  defp openssl_ok?(bin) do
    match?({_, 0}, System.cmd(bin, ["version"], stderr_to_stdout: true))
  rescue
    _ -> false
  end

  defp make_csr(tmp, subj, openssl) do
    key = Path.join(tmp, "client.key.pem")
    csr_path = Path.join(tmp, "client.csr.pem")

    {_, 0} =
      System.cmd(
        openssl,
        [
          "req",
          "-newkey",
          "rsa:2048",
          "-nodes",
          "-keyout",
          key,
          "-out",
          csr_path,
          "-subj",
          subj
        ],
        stderr_to_stdout: true
      )

    File.read!(csr_path)
  end

  defp cert_subject(tmp, cert_pem, openssl) do
    path = Path.join(tmp, "issued.cert.pem")
    File.write!(path, cert_pem)

    {subject, 0} =
      System.cmd(openssl, ["x509", "-in", path, "-noout", "-subject"], stderr_to_stdout: true)

    subject
  end
end
