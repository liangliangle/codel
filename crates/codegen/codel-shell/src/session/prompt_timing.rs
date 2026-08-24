pub struct PromptTiming;

impl PromptTiming {
    pub fn start() -> Self {
        Self
    }

    pub fn record_tool_prep(&mut self, _mcp_wait_ms: u64, _total_prep_ms: u64) {}

    pub fn record_first_token(&mut self, _ms: u64) {}

    pub fn emit(
        &self,
        _model_duration_ms: u64,
        _turn_index: u32,
        _mcp_count: u32,
        _mcp_tools: u32,
        _mcp_strategy: crate::session::mcp_servers::McpInitStrategy,
        _model_id: String,
    ) {
    }

    pub fn finish(&self) {}
}
