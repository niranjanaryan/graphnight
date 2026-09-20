defmodule GraphNight.Filter do
  @moduledoc """
  Represents a filter in a GraphNight query.
  """
  @enforce_keys [:field, :operator]
  defstruct [
    :field,
    :operator,
    :value,
    :or_condition
  ]

  @type t :: %__MODULE__{
    field: String.t(),
    operator: String.t(),  # "EQ", "NEQ", "GT", "GTE", "LT", "LTE", "LIKE", "ILIKE", "IN", "NOT_IN", "IS_NULL", "IS_NOT_NULL", "BETWEEN", "NOT_BETWEEN"
    value: term(),
    or_condition: boolean()
  }

  def new(field, operator, value, opts \\ []) do
    %__MODULE__{
      field: field,
      operator: operator,
      value: value,
      or_condition: Keyword.get(opts, :or_condition, false)
    }
  end

  def eq(field, value), do: new(field, "EQ", value)
  def neq(field, value), do: new(field, "NEQ", value)
  def gt(field, value), do: new(field, "GT", value)
  def gte(field, value), do: new(field, "GTE", value)
  def lt(field, value), do: new(field, "LT", value)
  def lte(field, value), do: new(field, "LTE", value)
  def like(field, value), do: new(field, "LIKE", value)
  def ilike(field, value), do: new(field, "ILIKE", value)
  def in(field, values), do: new(field, "IN", values)
  def not_in(field, values), do: new(field, "NOT_IN", values)
  def is_null(field), do: new(field, "IS_NULL", nil)
  def is_not_null(field), do: new(field, "IS_NOT_NULL", nil)
  def between(field, start, finish), do: new(field, "BETWEEN", [start, finish])
  def not_between(field, start, finish), do: new(field, "NOT_BETWEEN", [start, finish])
end