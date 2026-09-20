defmodule GraphNight.Model do
  @moduledoc """
  Represents a semantic model in GraphNight.
  """
  @enforce_keys [:name, :datasource]
  defstruct [
    :name,
    :datasource,
    :description,
    :measures,
    :dimensions,
    :time_dimensions,
    :joins
  ]

  @type t :: %__MODULE__{
    name: String.t(),
    datasource: String.t(),
    description: String.t() | nil,
    measures: [GraphNight.Measure.t()],
    dimensions: [GraphNight.Dimension.t()],
    time_dimensions: [GraphNight.TimeDimension.t()],
    joins: [GraphNight.Join.t()]
  }
end