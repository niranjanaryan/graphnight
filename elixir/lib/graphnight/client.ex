defmodule GraphNight.Client do
  @moduledoc """
  High-level client for GraphNight semantic layer.
  
  Provides a convenient Elixir API for working with GraphNight models and queries.
  """

  alias GraphNight.{
    Model, Measure, Dimension, TimeDimension, Join,
    DataSource, Query, SourceSpec, Filter, OrderBy,
    QueryResult
  }

  @typedoc """
  Opaque engine handle returned by `init/1`.
  """
  @type engine :: GraphNight.engine()

  @doc """
  Initialize a GraphNight engine with the given storage path.
  
  ## Example
  
      {:ok, engine} = GraphNight.Client.init("./graphnight_data")
  """
  @spec init(String.t()) :: {:ok, engine()} | {:error, String.t()}
  def init(storage_path) do
    try do
      {:ok, GraphNight.init_engine(storage_path)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  # DataSource operations
  @doc """
  Create a new datasource.
  """
  @spec create_datasource(engine(), DataSource.t()) :: {:ok, DataSource.t()} | {:error, String.t()}
  def create_datasource(engine, ds) do
    try do
      {:ok, GraphNight.create_datasource(engine, ds)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  List all datasources.
  """
  @spec list_datasources(engine()) :: {:ok, [DataSource.t()]} | {:error, String.t()}
  def list_datasources(engine) do
    try do
      {:ok, GraphNight.list_datasources(engine)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  Get a datasource by name.
  """
  @spec get_datasource(engine(), String.t()) :: {:ok, DataSource.t() | nil} | {:error, String.t()}
  def get_datasource(engine, name) do
    try do
      {:ok, GraphNight.get_datasource(engine, name)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  # Model operations
  @doc """
  Create a new model.
  """
  @spec create_model(engine(), Model.t()) :: {:ok, Model.t()} | {:error, String.t()}
  def create_model(engine, model) do
    try do
      {:ok, GraphNight.create_model(engine, model)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  List all models, optionally filtered by datasource.
  """
  @spec list_models(engine(), String.t() | nil) :: {:ok, [Model.t()]} | {:error, String.t()}
  def list_models(engine, datasource \\ nil) do
    try do
      {:ok, GraphNight.list_models(engine, datasource)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  Get a model by name.
  """
  @spec get_model(engine(), String.t(), String.t() | nil) :: {:ok, Model.t() | nil} | {:error, String.t()}
  def get_model(engine, name, datasource \\ nil) do
    try do
      {:ok, GraphNight.get_model(engine, name, datasource)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  Update an existing model.
  """
  @spec update_model(engine(), String.t(), Model.t()) :: {:ok, Model.t()} | {:error, String.t()}
  def update_model(engine, name, model) do
    try do
      {:ok, GraphNight.update_model(engine, name, model)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  Delete a model.
  """
  @spec delete_model(engine(), String.t(), String.t() | nil) :: {:ok, boolean()} | {:error, String.t()}
  def delete_model(engine, name, datasource \\ nil) do
    try do
      {:ok, GraphNight.delete_model(engine, name, datasource)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  # Query operations
  @doc """
  Execute a query.
  
  Returns a QueryResult with data, columns, SQL, and execution time.
  """
  @spec execute_query(engine(), Query.t(), boolean()) :: {:ok, QueryResult.t()} | {:error, String.t()}
  def execute_query(engine, query, dry_run \\ false) do
    try do
      {:ok, GraphNight.execute_query(engine, query, dry_run)}
    rescue
      e in RuntimeError -> {:error, e.message}
    end
  end

  @doc """
  Dry-run a query (returns SQL without executing).
  """
  @spec dry_run_query(engine(), Query.t()) :: {:ok, QueryResult.t()} | {:error, String.t()}
  def dry_run_query(engine, query) do
    execute_query(engine, query, true)
  end

  # Helper functions for building queries
  @doc """
  Build a simple aggregation query.
  
  ## Example
  
      query = GraphNight.Client.build_query(
        name: "orders",
        measures: [GraphNight.Measure.new("revenue:sum")],
        dimensions: [GraphNight.Dimension.new("status")],
        filters: [GraphNight.Filter.eq("status", "completed")]
      )
  """
  @spec build_query(keyword()) :: Query.t()
  def build_query(opts) do
    %Query{
      name: Keyword.get(opts, :name),
      source_model: Keyword.get(opts, :source_model),
      measures: Keyword.get(opts, :measures, []),
      dimensions: Keyword.get(opts, :dimensions, []),
      time_dimensions: Keyword.get(opts, :time_dimensions, []),
      filters: Keyword.get(opts, :filters, []),
      order: Keyword.get(opts, :order, []),
      limit: Keyword.get(opts, :limit),
      offset: Keyword.get(opts, :offset),
      whole_periods_only: Keyword.get(opts, :whole_periods_only),
      distinct_dimension_values: Keyword.get(opts, :distinct_dimension_values),
      stage_ref: Keyword.get(opts, :stage_ref)
    }
  end
end