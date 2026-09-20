defmodule GraphNight.SourceSpec do
  @moduledoc """
  Source model specification for queries.
  """
  @enforce_keys [:model]
  defstruct [
    :model,
    :datasource,
    :alias
  ]

  @type t :: %__MODULE__{
    model: String.t(),
    datasource: String.t() | nil,
    alias: String.t() | nil
  }

  def new(model, opts \\ []) do
    %__MODULE__{
      model: model,
      datasource: Keyword.get(opts, :datasource),
      alias: Keyword.get(opts, :alias)
    }
  end
end