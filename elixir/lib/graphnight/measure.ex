defmodule GraphNight.Measure do
  @moduledoc """
  Represents a measure (metric) in a GraphNight model.
  """
  @enforce_keys [:formula]
  defstruct [
    :formula,
    :label,
    :format,
    :aggregation
  ]

  @type t :: %__MODULE__{
    formula: String.t(),
    label: String.t() | nil,
    format: String.t() | nil,
    aggregation: String.t()  # "SUM", "AVG", "COUNT", "MIN", "MAX", "COUNT_DISTINCT", "CUSTOM"
  }

  @doc """
  Creates a simple measure.
  """
  def new(formula, opts \\ []) do
    %__MODULE__{
      formula: formula,
      label: Keyword.get(opts, :label),
      format: Keyword.get(opts, :format),
      aggregation: Keyword.get(opts, :aggregation, "SUM")
    }
  end
end