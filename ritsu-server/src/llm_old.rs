//! LLM client for multiple backends

#![allow(dead_code)] // TODO: Remove when all components are integrated

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::config::{LlmBackend, LlmConfig, TimeoutConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
}

#[derive(Deserialize)]
struct OllamaMessage {
    content: String,
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    content: String,
}

#[async_trait]
pub trait LlmBackendTrait: Send + Sync {
    async fn generate(&self, messages: &[Message], system_prompt: Option<&str>) -> Result<LlmResponse>;
}

pub struct LlmClient {
    backends: Vec<Box<dyn LlmBackendTrait>>,
    default_backend_index: usize,
}

impl LlmClient {
    pub fn new(config: &LlmConfig, timeout_config: &TimeoutConfig) -> Result<Self> {
        let mut backends: Vec<Box<dyn LlmBackendTrait>> = Vec::new();
        let mut default_backend_index = 0;

        for (idx, backend_config) in config.backends.iter().enumerate() {
            let backend: Box<dyn LlmBackendTrait> = if backend_config.endpoint.contains("ollama") {
                Box::new(OllamaBackend::new(backend_config, timeout_config)?)
            } else {
                Box::new(OpenAIBackend::new(backend_config, timeout_config)?)
            };
            
            backends.push(backend);

            if backend_config.name == config.default_backend {
                default_backend_index = idx;
            }
        }

        if backends.is_empty() {
            anyhow::bail!("No LLM backends configured");
        }

        info!("Initialized {} LLM backend(s)", backends.len());
        Ok(Self {
            backends,
            default_backend_index,
        })
    }

    pub async fn generate(&self, messages: &[Message], system_prompt: Option<&str>) -> Result<LlmResponse> {
        let backend = &self.backends[self.default_backend_index];
        backend.generate(messages, system_prompt).await
    }
}

// Ollama backend implementation
struct OllamaBackend {
    client: reqwest::Client,
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    fn new(config: &LlmBackend, timeout_config: &TimeoutConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout_config.llm_request())
            .build()?;

        Ok(Self {
            client,
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
        })
    }
}

#[async_trait]
impl LlmBackendTrait for OllamaBackend {
    async fn generate(&self, messages: &[Message], system_prompt: Option<&str>) -> Result<LlmResponse> {
        debug!("Generating with Ollama backend: {}", self.model);

        let mut full_messages = Vec::new();
        
        if let Some(prompt) = system_prompt {
            full_messages.push(Message {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }
        
        full_messages.extend_from_slice(messages);

        let request = OllamaRequest {
            model: self.model.clone(),
            messages: full_messages,
            stream: false,
        };

        let response = self.client
            .post(format!("{}/api/chat", self.endpoint))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        let ollama_response: OllamaResponse = response
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        Ok(LlmResponse {
            content: ollama_response.message.content,
            tool_calls: Vec::new(), // TODO: Parse tool calls when supported
        })
    }
}

// OpenAI-compatible backend implementation
struct OpenAIBackend {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAIBackend {
    fn new(config: &LlmBackend, timeout_config: &TimeoutConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout_config.llm_request())
            .build()?;

        let api_key = config.api_key_env.as_ref()
            .and_then(|env_var| std::env::var(env_var).ok());

        Ok(Self {
            client,
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            api_key,
        })
    }
}

#[async_trait]
impl LlmBackendTrait for OpenAIBackend {
    async fn generate(&self, messages: &[Message], system_prompt: Option<&str>) -> Result<LlmResponse> {
        debug!("Generating with OpenAI-compatible backend: {}", self.model);

        let mut full_messages = Vec::new();
        
        if let Some(prompt) = system_prompt {
            full_messages.push(Message {
                role: "system".to_string(),
                content: prompt.to_string(),
            });
        }
        
        full_messages.extend_from_slice(messages);

        let request = OpenAIRequest {
            model: self.model.clone(),
            messages: full_messages,
        };

        let mut req_builder = self.client
            .post(format!("{}/chat/completions", self.endpoint))
            .json(&request);

        if let Some(key) = &self.api_key {
            req_builder = req_builder.bearer_auth(key);
        }

        let response = req_builder
            .send()
            .await
            .context("Failed to send request to OpenAI-compatible API")?;

        let openai_response: OpenAIResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        let content = openai_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(LlmResponse {
            content,
            tool_calls: Vec::new(), // TODO: Parse tool calls when supported
        })
    }
}
