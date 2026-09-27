defmodule GraphNight.TimeDimension do
  @moduledoc """
  Represents a time dimension in a GraphNight model.
  """
  @enforce_keys [:dimension]
  defstruct [
    :dimension,
    :granularity,
    :label
  ]

  @type t :: %__MODULE__{
    dimension: String.t(),
    granularity: String.t()  # "SECOND", "MINUTE", "HOUR", "DAY", "WEEK", "MONTH", "QUARTER", "YEAR"
    label: String.t() | nil
  }

  def new(dimension, opts \\ []) do
    %__MODULE__{
      dimension: dimension,
      granularity: Keyword.get(opts, :granularity, "DAY"),
      label: Keyword.get(opts, :label)
    }
  end
end