use async_trait::async_trait;

/// Generic bot message abstraction
#[derive(Debug, Clone)]
pub struct BotMessage {
    pub text: String,
    pub session_id: String,
    pub reply_to_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BotCommand {
    pub name: String,
    pub description: String,
}

/// A generic abstraction layer for a chatbot platform.
#[async_trait]
pub trait ChatBotProvider: Send + Sync {
    /// Start the bot polling mechanism or webhook server, blocking the task.
    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Send a raw message back to a user session.
    async fn send_message(&self, session_id: &str, text: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Register slash commands or menus for the platform.
    async fn register_commands(&self, commands: Vec<BotCommand>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
