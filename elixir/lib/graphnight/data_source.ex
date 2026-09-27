defmodule GraphNight.DataSource do
  @moduledoc """
  Represents a data source connection in GraphNight.
  """
  @enforce_keys [:name, :driver, :connection_string]
  defstruct [
    :name,
    :driver,
    :connection_string,
    :description,
    :models,
    :pool_size
  ]

  @type t :: %__MODULE__{
    name: String.t(),
    driver: String.t(),  # "postgres", "mysql", "sqlite"
    connection_string: String.t(),
    description: String.t() | nil,
    models: [String.t()],
    pool_size: integer() | nil
  }

  def new(name, driver, connection_string, opts \\ []) do
    %__MODULE__{
      name: name,
      driver: driver,
      connection_string: connection_string,
      description: Keyword.get(opts, :description),
      models: Keyword.get(opts, :models, []),
      pool_size: Keyword.get(opts, :pool_size)
    }
  end
end