use serde_json::Value;

pub enum SecurityError {
    UnauthorizedAccess,
    InvalidParameters,
}

pub struct SecureString(String);

impl SecureString {
    pub fn new(val: String) -> Self {
        Self(val)
    }
}

pub trait SecurityVault: Send + Sync {
    fn validate_tool_call(&self, tool_name: &str, params: &Value) -> Result<(), SecurityError>;
    fn lease_temporary_credential(&self, tool_name: &str) -> Option<SecureString>;
}
