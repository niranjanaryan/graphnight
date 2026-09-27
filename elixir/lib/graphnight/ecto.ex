defmodule GraphNight.Ecto do
  @moduledoc """
  Ecto integration for GraphNight.
  
  Provides helpers for:
  - Using GraphNight models as Ecto schemas
  - Converting GraphNight queries to Ecto queries
  - Using GraphNight as a semantic layer on top of Ecto repositories
  
  Note: This is a lightweight integration. GraphNight uses its own SQL generator
  and does not require Ecto for query execution.
  """

  alias GraphNight.{Model, Measure, Dimension, TimeDimension, Query}

  @moduledoc """
  Convert a GraphNight Model to an Ecto Schema module definition.
  
  This generates Elixir code that can be used to create an Ecto schema
  matching the GraphNight model structure.
  """
  def model_to_ecto_schema(model, opts \\ []) do
    table_name = Keyword.get(opts, :table_name, model.name)
    primary_key = Keyword.get(opts, :primary_key, :id)
    
    fields = []
    
    # Add dimension fields
    for dim <- model.dimensions do
      fields = fields ++ [
        "field :#{dim.name}, :string"
      ]
    end
    
    # Add time dimension fields
    for td <- model.time_dimensions do
      fields = fields ++ [
        "field :#{td.dimension}, :utc_datetime"
      ]
    end
    
    # Add measure fields (these would typically be computed, not stored)
    for measure <- model.measures do
      fields = fields ++ [
        "# measure: #{measure.formula} (#{measure.aggregation})"
      ]
    end
    
    schema_code = """
    defmodule #{String.capitalize(table_name)} do
      use Ecto.Schema
      
      @primary_key {:#{primary_key}, :id, autogenerate: true}
      schema \"#{table_name}\" do
        #{Enum.join(fields, "\n        ")}
        
        timestamps()
      end
    end
    """
    
    schema_code
  end

  @doc """
  Convert a GraphNight Query to an Ecto.Query.
  
  This is a best-effort conversion. Complex GraphNight features like
  time dimensions, formulas, and multi-stage queries may not map directly.
  """
  def query_to_ecto(query, schema_module) do
    Ecto.Query.from(s in schema_module,
      select: build_select(query, s),
      where: build_where(query.filters, s),
      order_by: build_order(query.order, s),
      limit: query.limit,
      offset: query.offset,
      group_by: build_group_by(query, s)
    )
  end

  defp build_select(query, schema) do
    fields = []
    
    # Add dimensions
    for dim <- query.dimensions do
      fields = fields ++ [fragment("? as ?", field(schema, dim.name), dim.name)]
    end
    
    # Add time dimensions
    for td <- query.time_dimensions do
      granularity = String.downcase(td.granularity)
      fields = fields ++ [
        fragment("date_trunc(?, ?) as ?", ^granularity, field(schema, td.dimension), td.dimension)
      ]
    end
    
    # Add measures (as aggregations)
    for measure <- query.measures do
      agg = String.downcase(measure.aggregation)
      fields = fields ++ [
        fragment("#{agg}(?) as ?", field(schema, measure.formula), measure.formula)
      ]
    end
    
    map([struct: schema], fn x -> x end)
    |> Map.take(Enum.map(fields, &elem(&1, 2)))
  end

  defp build_where(filters, schema) do
    Enum.reduce(filters, [], fn filter, acc ->
      clause = case filter.operator do
        "EQ" -> fragment("? = ?", field(schema, filter.field), ^filter.value)
        "NEQ" -> fragment("? != ?", field(schema, filter.field), ^filter.value)
        "GT" -> fragment("? > ?", field(schema, filter.field), ^filter.value)
        "GTE" -> fragment("? >= ?", field(schema, filter.field), ^filter.value)
        "LT" -> fragment("? < ?", field(schema, filter.field), ^filter.value)
        "LTE" -> fragment("? <= ?", field(schema, filter.field), ^filter.value)
        "LIKE" -> fragment("? LIKE ?", field(schema, filter.field), ^filter.value)
        "ILIKE" -> fragment("? ILIKE ?", field(schema, filter.field), ^filter.value)
        "IN" -> fragment("? IN ?", field(schema, filter.field), ^filter.value)
        "NOT_IN" -> fragment("? NOT IN ?", field(schema, filter.field), ^filter.value)
        "IS_NULL" -> fragment("? IS NULL", field(schema, filter.field))
        "IS_NOT_NULL" -> fragment("? IS NOT NULL", field(schema, filter.field))
        "BETWEEN" -> fragment("? BETWEEN ? AND ?", field(schema, filter.field), ^Enum.at(filter.value, 0), ^Enum.at(filter.value, 1))
        "NOT_BETWEEN" -> fragment("? NOT BETWEEN ? AND ?", field(schema, filter.field), ^Enum.at(filter.value, 0), ^Enum.at(filter.value, 1))
        _ -> fragment("? = ?", field(schema, filter.field), ^filter.value)
      end
      
      if filter.or_condition do
        acc ++ [or: clause]
      else
        acc ++ [where: clause]
      end
    end)
  end

  defp build_order(order, schema) do
    Enum.map(order, fn o ->
      field_ref = field(schema, o.field)
      if o.descending do
        [desc: field_ref]
      else
        [asc: field_ref]
      end
    end)
  end

  defp build_group_by(query, schema) do
    fields = []
    
    for dim <- query.dimensions do
      fields = fields ++ [field(schema, dim.name)]
    end
    
    for td <- query.time_dimensions do
      granularity = String.downcase(td.granularity)
      fields = fields ++ [fragment("date_trunc(?, ?)", ^granularity, field(schema, td.dimension))]
    end
    
    fields
  end

  defp field(schema, name) do
    # Convert field access for Ecto
    {schema, ^name}
  end
end