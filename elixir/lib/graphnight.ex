defmodule GraphNight do
  @moduledoc """
  Elixir bindings for GraphNight - High-performance semantic layer with GraphQL/REST API.
  
  This module provides a native Elixir interface to the GraphNight Rust engine via Rustler NIFs.
  """

  use Rustler, otp_app: :graphnight, crate: :graphnight_elixir

  # NIF functions - implemented in Rust
  def init_engine(storage_path), do: :erlang.nif_error(:nif_not_loaded)
  def create_datasource(engine, ds), do: :erlang.nif_error(:nif_not_loaded)
  def list_datasources(engine), do: :erlang.nif_error(:nif_not_loaded)
  def get_datasource(engine, name), do: :erlang.nif_error(:nif_not_loaded)
  def create_model(engine, model), do: :erlang.nif_error(:nif_not_loaded)
  def list_models(engine, datasource \\ nil), do: :erlang.nif_error(:nif_not_loaded)
  def get_model(engine, name, datasource \\ nil), do: :erlang.nif_error(:nif_not_loaded)
  def update_model(engine, name, model), do: :erlang.nif_error(:nif_not_loaded)
  def delete_model(engine, name, datasource \\ nil), do: :erlang.nif_error(:nif_not_loaded)
  def execute_query(engine, query, dry_run \\ false), do: :erlang.nif_error(:nif_not_loaded)
  def dry_run_query(engine, query), do: :erlang.nif_error(:nif_not_loaded)
end