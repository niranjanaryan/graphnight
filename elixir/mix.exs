defmodule GraphNight.MixProject do
  use Mix.Project

  def project do
    [
      app: :graphnight,
      version: "1.0.0",
      elixir: "~> 1.14",
      start_permanent: Mix.env() == :prod,
      deps: deps(),
      description: "Elixir bindings for GraphNight semantic layer",
      package: package(),
      compilers: [:rustler] ++ Mix.compilers(),
      rustler_crates: [graphnight_elixir: {path: "../../crates/graphnight-elixir", features: ["nif"]}]
    ]
  end

  def application do
    [
      extra_applications: [:logger]
    ]
  end

  defp deps do
    [
      {:rustler, "~> 0.29", optional: true},
      {:jason, "~> 1.4"},
      {:ecto, "~> 3.10", optional: true},
      {:postgrex, "~> 0.17", optional: true}
    ]
  end

  defp package do
    [
      name: :graphnight,
      files: ["lib", "mix.exs", "README.md", "LICENSE"],
      maintainers: ["GraphNight Team"],
      licenses: ["Apache-2.0"],
      links: %{
        "GitHub" => "https://github.com/niranjanaryan/graphnight"
      }
    ]
  end
end