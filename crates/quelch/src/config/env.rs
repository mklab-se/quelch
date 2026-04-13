use thiserror::Error;

#[derive(Debug, Error)]
#[error("environment variable '{name}' not set (referenced in config)")]
pub struct EnvVarError {
    pub name: String,
}

/// Substitute `${VAR_NAME}` patterns in a string with environment variable values.
/// Returns an error if any referenced variable is not set.
pub fn substitute_env_vars(input: &str) -> Result<String, EnvVarError> {
    let result = shellexpand::env(input).map_err(|e| EnvVarError { name: e.var_name })?;
    Ok(result.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_env_var() {
        unsafe { std::env::set_var("QUELCH_TEST_VAR", "hello") };
        let result = substitute_env_vars("prefix-${QUELCH_TEST_VAR}-suffix").unwrap();
        assert_eq!(result, "prefix-hello-suffix");
    }

    #[test]
    fn returns_error_for_missing_var() {
        unsafe { std::env::remove_var("QUELCH_MISSING_VAR") };
        let result = substitute_env_vars("${QUELCH_MISSING_VAR}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.name, "QUELCH_MISSING_VAR");
    }

    #[test]
    fn no_substitution_needed() {
        let result = substitute_env_vars("plain string").unwrap();
        assert_eq!(result, "plain string");
    }

    #[test]
    fn multiple_vars() {
        unsafe {
            std::env::set_var("QUELCH_A", "one");
            std::env::set_var("QUELCH_B", "two");
        }
        let result = substitute_env_vars("${QUELCH_A}-${QUELCH_B}").unwrap();
        assert_eq!(result, "one-two");
    }
}
