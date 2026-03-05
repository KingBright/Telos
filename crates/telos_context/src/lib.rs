use telos_core::NodeResult;

pub struct LogEntry {
    pub timestamp: u64,
    pub message: String,
}

pub struct Document {
    pub doc_id: String,
    pub content: String,
}

pub struct RawContext {
    pub history_logs: Vec<LogEntry>,
    pub retrieved_docs: Vec<Document>,
}

pub struct SummaryNode {
    pub summary_text: String,
    pub children_ids: Vec<String>,
}

pub struct Fact {
    pub entity: String,
    pub relation: String,
    pub target: String,
}

pub struct ScopedContext {
    pub budget_tokens: usize,
    pub summary_tree: Vec<SummaryNode>,
    pub precise_facts: Vec<Fact>,
}

pub struct NodeRequirement {
    pub required_tokens: usize,
}

pub trait ContextManager: Send + Sync {
    fn compress_for_node(&self, raw: &RawContext, node_req: &NodeRequirement) -> ScopedContext;
    fn ingest_new_info(&mut self, info: NodeResult);
}
