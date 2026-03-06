use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub enum SecurityError {
    UnauthorizedAccess,
    InvalidParameters,
    ConfigurationError(String),
}

impl std::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecurityError::UnauthorizedAccess => write!(f, "Unauthorized access"),
            SecurityError::InvalidParameters => write!(f, "Invalid parameters"),
            SecurityError::ConfigurationError(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for SecurityError {}

#[derive(Debug)]
pub struct SecureString(String);

impl SecureString {
    pub fn new(val: String) -> Self {
        Self(val)
    }

    pub fn inner(&self) -> &str {
        &self.0
    }
}

#[async_trait]
pub trait SecurityVault: Send + Sync {
    async fn validate_tool_call(&self, role: &str, tool_name: &str, params: &Value) -> Result<(), SecurityError>;
    async fn lease_temporary_credential(&self, role: &str, tool_name: &str) -> Result<SecureString, SecurityError>;
}

pub mod vault;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::DefaultSecurityVault;
    use jsonwebtoken::{decode, DecodingKey, Validation};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct ToolClaims {
        sub: String,
        tool: String,
        iat: i64,
        exp: i64,
    }

    const MODEL_TEXT: &str = r#"
[request_definition]
r = sub, obj, act

[policy_definition]
p = sub, obj, act

[policy_effect]
e = some(where (p.eft == allow))

[matchers]
m = r.sub == p.sub && r.obj == p.obj && r.act == p.act
"#;

    const POLICY_TEXT: &str = r#"
p, agent_alpha, read_file, execute
p, agent_beta, write_file, execute
"#;

    const JWT_SECRET: &str = "super_secret_key_12345";

    async fn setup_vault() -> DefaultSecurityVault {
        DefaultSecurityVault::new(MODEL_TEXT, POLICY_TEXT, JWT_SECRET.to_string())
            .await
            .expect("Failed to create vault")
    }

    #[tokio::test]
    async fn test_abac_rejection() {
        let vault = setup_vault().await;

        // alpha should be allowed to read_file
        assert!(vault.validate_tool_call("agent_alpha", "read_file", &Value::Null).await.is_ok());

        // beta should be allowed to write_file
        assert!(vault.validate_tool_call("agent_beta", "write_file", &Value::Null).await.is_ok());

        // alpha should NOT be allowed to write_file
        let result = vault.validate_tool_call("agent_alpha", "write_file", &Value::Null).await;
        assert_eq!(result, Err(SecurityError::UnauthorizedAccess));

        // unknown agent should NOT be allowed to read_file
        let result = vault.validate_tool_call("unknown_agent", "read_file", &Value::Null).await;
        assert_eq!(result, Err(SecurityError::UnauthorizedAccess));
    }

    #[tokio::test]
    async fn test_successful_lease() {
        let vault = setup_vault().await;

        // Lease should succeed for authorized actions
        let token = vault.lease_temporary_credential("agent_alpha", "read_file").await;
        assert!(token.is_ok());
        let token = token.unwrap();

        // Lease should fail for unauthorized actions
        let result = vault.lease_temporary_credential("agent_alpha", "write_file").await;
        assert_eq!(result.unwrap_err(), SecurityError::UnauthorizedAccess);
    }

    #[tokio::test]
    async fn test_jwt_expiration() {
        let vault = setup_vault().await;

        let token = vault.lease_temporary_credential("agent_alpha", "read_file")
            .await
            .expect("Failed to lease credential");

        // Decode and verify JWT
        let token_data = decode::<ToolClaims>(
            token.inner(),
            &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
            &Validation::default(),
        ).expect("Failed to decode JWT");

        assert_eq!(token_data.claims.sub, "agent_alpha");
        assert_eq!(token_data.claims.tool, "read_file");

        let iat = token_data.claims.iat;
        let exp = token_data.claims.exp;

        // Expiration should be 5 minutes (300 seconds) after issued
        assert_eq!(exp - iat, 300);
    }
}
