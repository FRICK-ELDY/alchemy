defmodule AlchemyAuthClient.MixProject do
  use Mix.Project

  def project do
    [
      app: :alchemy_auth_client,
      version: "0.1.0",
      elixir: "~> 1.19",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      package: package(),
      description: "Shared auth-server JWKS cache + user JWT verify for Alchemy world/assets"
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp elixirc_paths(_), do: ["lib"]

  defp deps do
    [
      {:joken, "~> 2.6"},
      {:jose, "~> 1.11"},
      {:req, "~> 0.5"},
      {:bypass, "~> 2.1", only: :test}
    ]
  end

  defp package do
    [
      licenses: ["MIT"],
      links: %{"GitHub" => "https://github.com/FRICK-ELDY/alchemy"}
    ]
  end
end
