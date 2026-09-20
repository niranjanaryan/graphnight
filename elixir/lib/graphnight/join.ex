defmodule GraphNight.Join do
  @moduledoc """
  Represents a join between models in GraphNight.
  """
  @enforce_keys [:name, :model]
  defstruct [
    :name,
    :model,
    :join_type,
    :on,
    :alias
  ]

  @type t :: %__MODULE__{
    name: String.t(),
    model: String.t(),
    join_type: String.t(),  # "INNER", "LEFT", "RIGHT", "FULL"
    on: [{String.t(), String.t()}],
    alias: String.t() | nil
  }

  def new(name, model, opts \\ []) do
    %__MODULE__{
      name: name,
      model: model,
      join_type: Keyword.get(opts, :join_type, "LEFT"),
      on: Keyword.get(opts, :on, []),
      alias: Keyword.get(opts, :alias)
    }
  end
end