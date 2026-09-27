defmodule GraphNight.OrderBy do
  @moduledoc """
  Represents an ORDER BY clause in a GraphNight query.
  """
  @enforce_keys [:field]
  defstruct [
    :field,
    :descending
  ]

  @type t :: %__MODULE__{
    field: String.t(),
    descending: boolean()
  }

  def new(field, descending \\ false) do
    %__MODULE__{
      field: field,
      descending: descending
    }
  end

  def asc(field), do: new(field, false)
  def desc(field), do: new(field, true)
end