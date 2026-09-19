use crate::context::GraphQLContext;
use async_graphql::{Context, Error, Result};
use graphnight_core::models::Query as CoreQuery;
use graphnight_core::security::{PolicyEnforcer, SessionPolicy};

/// Require an authenticated user on the request context.
pub fn require_user(ctx: &Context<'_>) -> Result<String> {
    let gql = ctx.data::<GraphQLContext>()?;
    gql.user_id
        .clone()
        .ok_or_else(|| Error::new("Authentication required"))
}

/// Require an admin user (datasource / model write operations).
pub fn require_admin(ctx: &Context<'_>) -> Result<String> {
    let gql = ctx.data::<GraphQLContext>()?;
    if !gql.is_admin {
        return Err(Error::new("Admin privileges required"));
    }
    gql.user_id
        .clone()
        .ok_or_else(|| Error::new("Authentication required"))
}

/// Apply session policy to a mutable query when present.
pub fn enforce_policy(
    ctx: &Context<'_>,
    query: &mut CoreQuery,
    model_name: &str,
    datasource: &str,
) -> Result<()> {
    let gql = ctx.data::<GraphQLContext>()?;
    let policy = gql
        .session_policy
        .clone()
        .unwrap_or_else(SessionPolicy::new);
    let enforcer = PolicyEnforcer::new(policy);
    enforcer
        .check_model_access(model_name)
        .map_err(|e| Error::new(e.to_string()))?;
    enforcer
        .check_datasource_access(datasource)
        .map_err(|e| Error::new(e.to_string()))?;
    enforcer.apply_forced_filters(query);
    enforcer.apply_rls(query);
    enforcer.enforce_row_limit(query);
    Ok(())
}

/// When auth is required on the server, reject anonymous GraphQL operations.
pub fn reject_if_auth_required(ctx: &Context<'_>) -> Result<()> {
    let gql = ctx.data::<GraphQLContext>()?;
    if gql.auth_required && gql.user_id.is_none() {
        return Err(Error::new(
            "Authentication required (provide Authorization: Bearer <key> or X-API-Key)",
        ));
    }
    Ok(())
}

/// Admin gate that only applies when the server is in auth-required mode.
pub fn require_admin_if_auth(ctx: &Context<'_>) -> Result<()> {
    let gql = ctx.data::<GraphQLContext>()?;
    if !gql.auth_required {
        return Ok(());
    }
    require_admin(ctx).map(|_| ())
}

/// User gate that only applies when the server is in auth-required mode.
pub fn require_user_if_auth(ctx: &Context<'_>) -> Result<()> {
    let gql = ctx.data::<GraphQLContext>()?;
    if !gql.auth_required {
        return Ok(());
    }
    require_user(ctx).map(|_| ())
}
