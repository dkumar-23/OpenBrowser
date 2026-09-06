// runtime-mcp: MCP adapter (server + client) implementing InteractionAdapter.
//
// Per context.md §13: "MCP/CDP as adapters, never core."
// This adapter plugs into AdapterRegistry (runtime-interaction) so the CLI
// and kernel can dispatch MCP-capable actions through the same interface.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use runtime_interaction::{
    InteractionAdapter, AdapterDescriptor, AdapterKind,
    AdapterParams, AdapterResult, TaskInfo,
};
use runtime_auth::AgentIdentity;
use runtime_policy::{PolicyEngine, CapabilitySet, Decision};
use runtime_observability::{Observability, ReplayEvent};

/// MCP server interface — hosts capabilities to external MCP clients.
#[async_trait]
pub trait McpServer: Send + Sync + std::fmt::Debug {
    async fn list_capabilities(&self) -> Vec<String>;
    async fn invoke(
        &self,
        capability: &str,
        agent: &AgentIdentity,
        params: serde_json::Value,
    ) -> AdapterResult;
}

/// MCP client interface — connects to remote MCP servers.
#[async_trait]
pub trait McpClient: Send + Sync + std::fmt::Debug {
    async fn connect(&self, endpoint: &str) -> anyhow::Result<()>;
    async fn call_tool(
        &self,
        name: &str,
        args: HashMap<String, String>,
    ) -> anyhow::Result<serde_json::Value>;
}

/// McpAdapter — implements InteractionAdapter for MCP protocol.
/// Enables adapter registry to include MCP capabilities.
#[derive(Debug)]
pub struct McpAdapter {
    server: Option<Box<dyn McpServer>>,
    client: Option<Box<dyn McpClient>>,
    policy: Arc<PolicyEngine>,
    observability: Arc<dyn Observability>,
}

impl McpAdapter {
    pub fn new(
        policy: Arc<PolicyEngine>,
        observability: Arc<dyn Observability>,
    ) -> Self {
        Self {
            server: None,
            client: None,
            policy,
            observability,
        }
    }
    pub fn with_server(mut self, s: Box<dyn McpServer>) -> Self {
        self.server = Some(s);
        self
    }
    pub fn with_client(mut self, c: Box<dyn McpClient>) -> Self {
        self.client = Some(c);
        self
    }
}

#[async_trait]
impl InteractionAdapter for McpAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            kind: AdapterKind::Mcp,
            handles: vec![
                "search_web".into(),
                "extract_page".into(),
                "authenticate".into(),
                "submit_form".into(),
                "purchase".into(),
                "schedule".into(),
                "mcp.invoke".into(),
            ],
        }
    }

    async fn execute(
        &self,
        agent: &AgentIdentity,
        caps: &CapabilitySet,
        info: &TaskInfo,
        params: &AdapterParams,
    ) -> AdapterResult {
        // Policy enforcement before network/protocol call (CF-1 + CF-2).
        let decision = self.policy.check_with_caps(agent, caps, "mcp.invoke");
        match decision {
            Decision::Allow => {
                // Delegate to server or client based on params.
                let result_body = match params {
                    AdapterParams::Mcp { tool, args } => {
                        if let Some(s) = &self.server {
                            s.invoke(tool, agent, serde_json::json!(args)).await
                        } else {
                            AdapterResult::Error {
                                message: "MCP server not configured".into(),
                                replay_sequence: 0,
                            }
                        }
                    },
                    _ => AdapterResult::Error {
                        message: "MCP adapter expects AdapterParams::Mcp".into(),
                        replay_sequence: 0,
                    },
                };
                result_body
            },
            Decision::Deny { reason } => {
                let event = ReplayEvent {
                    sequence: 0,
                    event_type: "policy_denied".into(),
                    task_id: info.task_id,
                    agent_id: agent.agent_id.0,
                    result_summary: reason.clone(),
                    timestamp: chrono::Utc::now(),
                };
                let replay_seq = self.observability.record_replay(event);
                self.observability.metric("mcp_policy_denied", 1.0, &[("capability", "mcp.invoke")]);
                AdapterResult::Denied { reason, replay_sequence: replay_seq }
            },
        }
    }
}

/// Default MCP server implementation (stub).
#[derive(Default, Debug)]
pub struct DefaultMcpServer;

#[async_trait]
impl McpServer for DefaultMcpServer {
    async fn list_capabilities(&self) -> Vec<String> {
        vec![
            "search_web".into(),
            "extract_page".into(),
            "authenticate".into(),
        ]
    }
    async fn invoke(
        &self,
        _capability: &str,
        _agent: &AgentIdentity,
        params: serde_json::Value,
    ) -> AdapterResult {
        AdapterResult::Success {
            response: format!("{{\"mcp\":\"ok\",\"params\":{}}}", params),
            replay_sequence: 1,
        }
    }
}

/// Default MCP client (stub).
#[derive(Default, Debug)]
pub struct DefaultMcpClient {
    endpoint: Option<String>,
}

impl DefaultMcpClient {
    pub fn new() -> Self { Self { endpoint: None } }
    pub fn with_endpoint(mut self, ep: &str) -> Self {
        self.endpoint = Some(ep.into());
        self
    }
}

#[async_trait]
impl McpClient for DefaultMcpClient {
    async fn connect(&self, endpoint: &str) -> anyhow::Result<()> {
        tracing::info!(mcp_endpoint = %endpoint, "MCP client connecting");
        Ok(())
    }
    async fn call_tool(
        &self,
        name: &str,
        args: HashMap<String, String>,
    ) -> anyhow::Result<serde_json::Value> {
        tracing::info!(mcp_tool = %name, ?args, "MCP client calling tool");
        Ok(serde_json::json!({
            "tool": name,
            "args": args,
            "status": "ok",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_auth::{AgentIdentity, HumanId};
    use std::sync::Arc;

    fn make_identity() -> AgentIdentity {
        AgentIdentity::new(HumanId(uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn test_mcp_adapter_policy_denied() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let caps = runtime_policy::CapabilitySet::new();
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("mcp.invoke");
            p
        });
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let adapter = McpAdapter::new(policy, obs);
        let params = AdapterParams::Mcp {
            tool: "search_web".into(),
            args: HashMap::new(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Denied { .. }));
    }

    #[tokio::test]
    async fn test_mcp_adapter_policy_allowed_with_default_server() {
        let agent = make_identity();
        let info = runtime_interaction::TaskInfo::new(uuid::Uuid::new_v4(), agent.agent_id.0);
        let mut caps = runtime_policy::CapabilitySet::new();
        caps.grant(runtime_policy::Capability::new("mcp.invoke", runtime_policy::Scope::Read, None));
        let policy = Arc::new({
            let mut p = runtime_policy::PolicyEngine::new();
            p.add_capability("mcp.invoke");
            p
        });
        let obs = Arc::new(runtime_observability::TraceObservability::without_replay());
        let adapter = McpAdapter::new(policy, obs).with_server(Box::new(DefaultMcpServer));
        let params = AdapterParams::Mcp {
            tool: "search_web".into(),
            args: HashMap::new(),
        };
        let result = adapter.execute(&agent, &caps, &info, &params).await;
        assert!(matches!(result, AdapterResult::Success { .. }));
    }

    #[test]
    fn test_default_mcp_client() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = DefaultMcpClient::new().with_endpoint("ws://localhost:3000");
            let result = client.connect("ws://localhost:3000").await;
            assert!(result.is_ok());
            let tool_result = client.call_tool("search_web", HashMap::new()).await;
            assert!(tool_result.is_ok());
        });
    }
}
