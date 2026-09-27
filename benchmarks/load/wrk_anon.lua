wrk.method = "POST"
wrk.path = "/api/v1/query"
wrk.headers["Content-Type"] = "application/json"
wrk.body = '{"name":"orders","source_model":null,"measures":[{"formula":"amount_usd","label":null,"format":null,"aggregation":"sum"}],"dimensions":null,"time_dimensions":null,"filters":null,"order":null,"limit":10,"offset":null,"whole_periods_only":null,"distinct_dimension_values":null,"stage_ref":null}'