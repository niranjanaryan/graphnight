//! Connection-string secret references (`env:VARNAME`).

use std::env;

/// Prefix for environment-variable secret references.
pub const ENV_SECRET_PREFIX: &str = "env:";

/// True when `GRAPHNIGHT_REQUIRE_SECRET_REFS` is `1` / `true`.
pub fn require_secret_refs_enabled() -> bool {
    env::var("GRAPHNIGHT_REQUIRE_SECRET_REFS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Returns true if `value` is an `env:VARNAME` reference.
pub fn is_env_secret_ref(value: &str) -> bool {
    value
        .strip_prefix(ENV_SECRET_PREFIX)
        .map(|name| !name.is_empty() && !name.contains(':'))
        .unwrap_or(false)
}

/// Extract the env var name from an `env:VARNAME` reference.
pub fn env_secret_var_name(value: &str) -> Option<&str> {
    value.strip_prefix(ENV_SECRET_PREFIX).and_then(|name| {
        let name = name.trim();
        if name.is_empty() || name.contains(':') {
            None
        } else {
            Some(name)
        }
    })
}

/// Resolve `env:VARNAME` to the environment value; pass through other strings.
pub fn resolve_connection_string(value: &str) -> Result<String, String> {
    if let Some(var) = env_secret_var_name(value) {
        env::var(var).map_err(|_| {
            format!("secret reference env:{var} is set but environment variable '{var}' is missing")
        })
    } else {
        Ok(value.to_string())
    }
}

/// When `GRAPHNIGHT_REQUIRE_SECRET_REFS` is enabled, require `env:VARNAME` form.
pub fn validate_connection_string_input(value: &str) -> Result<(), String> {
    if !require_secret_refs_enabled() {
        return Ok(());
    }
    if is_env_secret_ref(value) {
        Ok(())
    } else {
        Err(
            "GRAPHNIGHT_REQUIRE_SECRET_REFS=1: connection_string must be env:VARNAME \
             (e.g. env:GRAPHNIGHT_DATASOURCE_DEMO)"
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_env_refs() {
        assert!(is_env_secret_ref("env:GRAPHNIGHT_DATASOURCE_DEMO"));
        assert!(!is_env_secret_ref("postgresql://localhost/db"));
        assert!(!is_env_secret_ref("env:"));
        assert!(!is_env_secret_ref("env:FOO:BAR"));
    }

    #[test]
    fn resolve_passthrough() {
        assert_eq!(
            resolve_connection_string("sqlite:./demo.db").unwrap(),
            "sqlite:./demo.db"
        );
    }

    #[test]
    fn resolve_env_ref() {
        env::set_var("GRAPHNIGHT_TEST_SECRET_REF", "postgresql://resolved/db");
        let got = resolve_connection_string("env:GRAPHNIGHT_TEST_SECRET_REF").unwrap();
        assert_eq!(got, "postgresql://resolved/db");
        env::remove_var("GRAPHNIGHT_TEST_SECRET_REF");
    }

    #[test]
    fn resolve_missing_env_errors() {
        env::remove_var("GRAPHNIGHT_TEST_SECRET_MISSING");
        let err = resolve_connection_string("env:GRAPHNIGHT_TEST_SECRET_MISSING").unwrap_err();
        assert!(err.contains("missing"));
    }

    #[test]
    fn validate_when_required() {
        env::set_var("GRAPHNIGHT_REQUIRE_SECRET_REFS", "1");
        assert!(validate_connection_string_input("env:FOO").is_ok());
        assert!(validate_connection_string_input("postgresql://x").is_err());
        env::remove_var("GRAPHNIGHT_REQUIRE_SECRET_REFS");
        assert!(validate_connection_string_input("postgresql://x").is_ok());
    }
}
