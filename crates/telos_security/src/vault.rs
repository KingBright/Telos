use crate::{SecureString, SecurityError, SecurityVault};
use async_trait::async_trait;
use casbin::{CoreApi, DefaultModel, Enforcer, MgmtApi, MemoryAdapter};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
struct ToolClaims {
    sub: String,       // role
    tool: String,      // tool name
    iat: i64,          // issued at
    exp: i64,          // expires at
}

pub struct DefaultSecurityVault {
    enforcer: tokio::sync::Mutex<Enforcer>,
    jwt_secret: String,
}

impl DefaultSecurityVault {
    pub async fn new(model_text: &str, policy_text: &str, jwt_secret: String) -> Result<Self, SecurityError> {
        let model = DefaultModel::from_str(model_text)
            .await
            .map_err(|e| SecurityError::ConfigurationError(e.to_string()))?;

        let adapter = MemoryAdapter::default();
        let mut enforcer = Enforcer::new(model, adapter)
            .await
            .map_err(|e| SecurityError::ConfigurationError(e.to_string()))?;

        // Assuming policy_text is just simple CSV lines for now
        for line in policy_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() > 1 && parts[0] == "p" {
                let rule: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                enforcer.add_policy(rule).await.map_err(|e| SecurityError::ConfigurationError(e.to_string()))?;
            }
        }

        Ok(Self {
            enforcer: tokio::sync::Mutex::new(enforcer),
            jwt_secret,
        })
    }
}

#[async_trait]
impl SecurityVault for DefaultSecurityVault {
    async fn validate_tool_call(
        &self,
        role: &str,
        tool_name: &str,
        _params: &Value,
    ) -> Result<(), SecurityError> {
        let enforcer = self.enforcer.lock().await;
        let ok = enforcer.enforce((role, tool_name, "execute"))
            .map_err(|e| SecurityError::ConfigurationError(e.to_string()))?;

        if ok {
            Ok(())
        } else {
            Err(SecurityError::UnauthorizedAccess)
        }
    }

    async fn lease_temporary_credential(
        &self,
        role: &str,
        tool_name: &str,
    ) -> Result<SecureString, SecurityError> {
        self.validate_tool_call(role, tool_name, &Value::Null).await?;

        let now = Utc::now();
        // Tight TTL logic: 5 minutes expiration
        let exp = now + Duration::minutes(5);

        let claims = ToolClaims {
            sub: role.to_string(),
            tool: tool_name.to_string(),
            iat: now.timestamp(),
            exp: exp.timestamp(),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        ).map_err(|e| SecurityError::ConfigurationError(e.to_string()))?;

        Ok(SecureString::new(token))
    }
}
