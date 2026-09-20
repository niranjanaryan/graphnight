defmodule GraphNight.Dimension do
  @moduledoc """
  Represents a dimension in a GraphNight model.
  """
  @enforce_keys [:name]
  defstruct [
    :name,
    :label
  ]

  @type t :: %__MODULE__{
    name: String.t(),
    label: String.t() | nil
  }

  def new(name, opts \\ []) do
    %__MODULE__{
      name: name,
      label: Keyword.get(opts, :label)
    }
  end
end