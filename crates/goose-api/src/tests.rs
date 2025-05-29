#[cfg(test)]
mod tests {
    use super::*;
    use goose::message::{Message, MessageContent};
    use goose::model::ModelConfig;
    use goose::providers::{
        base::{Provider, ProviderMetadata, ProviderUsage, Usage},
        errors::ProviderError,
    };
    use mcp_core::tool::Tool;
    use std::sync::Arc;
    use tempfile::TempDir;
    use warp::reply::Reply;
    use goose::session::{self, Identifier};
    use uuid::Uuid;
    use hyper::body;

    #[derive(Clone)]
    struct ContextProvider {
        model_config: ModelConfig,
    }

    #[async_trait::async_trait]
    impl Provider for ContextProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn complete(
            &self,
            system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            if system.contains("summarizing") {
                Ok((
                    Message::user().with_text("summary"),
                    ProviderUsage::new("mock".to_string(), Usage::default()),
                ))
            } else {
                Err(ProviderError::ContextLengthExceeded("too long".to_string()))
            }
        }
    }

    async fn setup() -> (TempDir, Uuid) {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let provider = Arc::new(ContextProvider {
            model_config: ModelConfig::new("test".to_string()),
        });
        let agent = AGENT.lock().await;
        agent.update_provider(provider).await.unwrap();
        drop(agent);

        let req = SessionRequest {
            prompt: "start".repeat(1000),
        };
        let reply = start_session_handler(req, "key".to_string()).await.unwrap();
        let resp = reply.into_response();
        let body = body::to_bytes(resp.into_body()).await.unwrap();
        let start: StartSessionResponse = serde_json::from_slice(&body).unwrap();
        (tmp, start.session_id)
    }

    #[tokio::test]
    async fn build_routes_compiles() {
        let _routes = build_routes("test-key".to_string());
    }

    #[tokio::test]
    async fn summarizes_large_history_on_start() {
        let (tmp, session_id) = setup().await;

        let session_path = session::get_path(Identifier::Name(session_id.to_string()));
        let messages = session::read_messages(&session_path).unwrap();
        assert!(messages.iter().any(|m| m.as_concat_text().contains("summary")));
        drop(tmp);
    }

    #[tokio::test]
    async fn summarizes_large_history_on_reply() {
        let (tmp, session_id) = setup().await;

        let req = SessionReplyRequest {
            session_id,
            prompt: "reply".repeat(1000),
        };
        let reply = reply_session_handler(req, "key".to_string()).await.unwrap();
        let resp = reply.into_response();
        let body = body::to_bytes(resp.into_body()).await.unwrap();
        let api: ApiResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(api.status, "warning");

        let session_path = session::get_path(Identifier::Name(session_id.to_string()));
        let messages = session::read_messages(&session_path).unwrap();
        assert!(messages
            .iter()
            .all(|m| !matches!(m.content.first(), Some(MessageContent::ContextLengthExceeded(_)))));
        drop(tmp);
    }
}
