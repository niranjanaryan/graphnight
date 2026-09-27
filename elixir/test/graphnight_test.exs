defmodule GraphNightTest do
  use ExUnit.Case
  alias GraphNight.{Model, Measure, Dimension, TimeDimension, DataSource, Query, Filter}

  @tag :integration
  test "initializes engine" do
    {:ok, engine} = GraphNight.Client.init("./test_data")
    assert engine != nil
  end

  @tag :integration
  test "creates and lists datasources" do
    {:ok, engine} = GraphNight.Client.init("./test_data")
    
    ds = DataSource.new("test_pg", "postgres", "postgresql://localhost/test")
    {:ok, _ds} = GraphNight.Client.create_datasource(engine, ds)
    
    {:ok, datasources} = GraphNight.Client.list_datasources(engine)
    assert length(datasources) >= 1
    assert Enum.any?(datasources, fn d -> d.name == "test_pg" end)
  end

  @tag :integration
  test "creates and queries model" do
    {:ok, engine} = GraphNight.Client.init("./test_data")
    
    model = %Model{
      name: "test_orders",
      datasource: "test_pg",
      measures: [
        %Measure{formula: "revenue:sum", aggregation: "SUM"},
        %Measure{formula: "count:count", aggregation: "COUNT"}
      ],
      dimensions: [
        %Dimension{name: "status"},
        %Dimension{name: "store_id"}
      ],
      time_dimensions: [
        %TimeDimension{dimension: "created_at", granularity: "DAY"}
      ],
      joins: []
    }
    
    {:ok, _model} = GraphNight.Client.create_model(engine, model)
    
    {:ok, models} = GraphNight.Client.list_models(engine, "test_pg")
    assert Enum.any?(models, fn m -> m.name == "test_orders" end)
  end

  @tag :integration
  test "dry run query generates SQL" do
    {:ok, engine} = GraphNight.Client.init("./test_data")
    
    query = Query.new(
      name: "test_orders",
      measures: [Measure.new("revenue:sum")],
      dimensions: [Dimension.new("status")],
      filters: [Filter.eq("status", "completed")]
    )
    
    {:ok, result} = GraphNight.Client.dry_run_query(engine, query)
    
    assert result.sql != nil
    assert String.contains?(result.sql, "SELECT")
    assert String.contains?(result.sql, "revenue")
    assert String.contains?(result.sql, "status")
  end
end