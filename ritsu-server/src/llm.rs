//! LLM client with tool calling support using the `llm` crate

use anyhow::{Context, Result};
use llm::chat::{ChatMessage, ChatRole, FunctionTool, ParameterProperty, ParametersSchema, Tool, ToolChoice};
use llm::builder::{LLMBackend, LLMBuilder};
use llm::LLMProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::config::{LlmBackend, LlmConfig, TimeoutConfig};
use crate::tools::{ToolRegistry, ToolParameter};

/// Message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Tool call from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// LLM response with optional tool calls
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Unified LLM client supporting multiple backends
pub struct LlmClient {
    provider: Arc<dyn LLMProvider>,
    tool_registry: Arc<ToolRegistry>,
}

impl LlmClient {
    pub fn new(
        config: &LlmConfig,
        _timeout_config: &TimeoutConfig,
        tool_registry: Arc<ToolRegistry>,
    ) -> Result<Self> {
        // Use the first backend for now (can be extended to support multiple)
        let backend = config.backends.first()
            .context("No LLM backends configured")?;

        let provider = Self::create_provider(backend)?;

        Ok(Self {
            provider: Arc::new(provider),
            tool_registry,
        })
    }

    fn create_provider(backend: &LlmBackend) -> Result<Box<dyn LLMProvider>> {
        // Detect provider from endpoint
        let provider_type = if backend.endpoint.contains("openai") || backend.endpoint.contains("api.openai.com") {
            LLMBackend::OpenAI
        } else if backend.endpoint.contains("anthropic") || backend.endpoint.contains("api.anthropic.com") {
            LLMBackend::Anthropic
        } else if backend.endpoint.contains("localhost") || backend.endpoint.contains("ollama") {
            LLMBackend::Ollama
        } else {
            // Default to ollama for custom endpoints
            LLMBackend::Ollama
        };

        info!("Initializing {:?} provider with model: {} (endpoint: {})", 
            provider_type, backend.model, backend.endpoint);

        // Get API key from environment if specified
        let api_key = backend.api_key_env.as_ref()
            .and_then(|env_var| std::env::var(env_var).ok());

        let mut builder = LLMBuilder::new()
            .backend(provider_type)
            .model(&backend.model);

        if let Some(key) = api_key {
            builder = builder.api_key(&key);
        }

        if !backend.endpoint.is_empty() {
            builder = builder.api_base(&backend.endpoint);
        }

        builder.build().context("Failed to build LLM provider")
    }

    /// Generate response with optional tool calling support
    pub async fn generate(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
    ) -> Result<LlmResponse> {
        self.generate_with_tools(messages, system_prompt, true).await
    }

    /// Generate response with control over tool usage
    pub async fn generate_with_tools(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        enable_tools: bool,
    ) -> Result<LlmResponse> {
        // Convert our messages to llm crate format
        let mut chat_messages = Vec::new();
        
        // Add system prompt if provided
        if let Some(system) = system_prompt {
            chat_messages.push(
                ChatMessage::new()
                    .role(ChatRole::System)
                    .content(system)
                    .build()
            );
        }

        // Add conversation messages
        for msg in messages {
            let role = match msg.role.as_str() {
                "system" => ChatRole::System,
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => {
                    warn!("Unknown role '{}', defaulting to user", msg.role);
                    ChatRole::User
                }
            };

            chat_messages.push(
                ChatMessage::new()
                    .role(role)
                    .content(&msg.content)
                    .build()
            );
        }

        // Prepare tools if enabled
        let tools = if enable_tools {
            Some(self.convert_tools_for_llm().await?)
        } else {
            None
        };

        debug!("Sending {} messages to LLM (tools: {})", 
            chat_messages.len(), 
            tools.as_ref().map_or(0, |t| t.len())
        );

        // Make the chat request
        let response = if let Some(tool_list) = tools {
            self.provider.chat()
                .messages(chat_messages)
                .tools(tool_list)
                .tool_choice(ToolChoice::Auto)
                .send()
                .await
                .context("Failed to send chat request with tools")?
        } else {
            self.provider.chat()
                .messages(chat_messages)
                .send()
                .await
                .context("Failed to send chat request")?
        };

        // Extract content
        let content = response.choices()
            .first()
            .and_then(|choice| choice.message())
            .map(|msg| msg.content().to_string())
            .unwrap_or_default();

        // Extract tool calls
        let tool_calls = response.choices()
            .first()
            .and_then(|choice| choice.message())
            .and_then(|msg| msg.tool_calls())
            .map(|calls| {
                calls.iter()
                    .filter_map(|call| {
                        let function = call.function();
                        Some(ToolCall {
                            name: function.name().to_string(),
                            arguments: serde_json::from_str(function.arguments()).ok()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if !tool_calls.is_empty() {
            debug!("LLM requested {} tool call(s)", tool_calls.len());
        }

        Ok(LlmResponse {
            content,
            tool_calls,
        })
    }

    /// Execute tool calls and return results
    pub async fn execute_tool_calls(&self, tool_calls: &[ToolCall]) -> Vec<(String, String)> {
        let mut results = Vec::new();

        for call in tool_calls {
            info!("Executing tool: {} with args: {}", call.name, call.arguments);

            // Convert JSON arguments to HashMap<String, String>
            let args = match call.arguments.as_object() {
                Some(obj) => {
                    obj.iter()
                        .filter_map(|(k, v)| {
                            let value = match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            Some((k.clone(), value))
                        })
                        .collect()
                }
                None => HashMap::new(),
            };

            // Execute the tool
            match self.tool_registry.execute(&call.name, args).await {
                Ok(result) => {
                    let result_str = if result.success {
                        format!("Success: {}", result.output)
                    } else {
                        format!("Error: {}", result.output)
                    };
                    info!("Tool {} result: {}", call.name, result_str);
                    results.push((call.name.clone(), result_str));
                }
                Err(e) => {
                    let error_str = format!("Tool execution failed: {}", e);
                    warn!("Tool {} error: {}", call.name, error_str);
                    results.push((call.name.clone(), error_str));
                }
            }
        }

        results
    }

    /// Generate response with automatic tool execution loop
    pub async fn generate_with_tool_execution(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        max_iterations: usize,
    ) -> Result<LlmResponse> {
        let mut current_messages = messages.to_vec();
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                warn!("Reached max iterations ({}) for tool execution loop", max_iterations);
                break;
            }

            // Generate response with tools
            let response = self.generate_with_tools(&current_messages, system_prompt, true).await?;

            // If no tool calls, we're done
            if response.tool_calls.is_empty() {
                return Ok(response);
            }

            // Execute tool calls
            let tool_results = self.execute_tool_calls(&response.tool_calls).await;

            // Add assistant message with tool calls
            if !response.content.is_empty() {
                current_messages.push(Message {
                    role: "assistant".to_string(),
                    content: response.content.clone(),
                });
            }

            // Add tool results as user messages
            for (tool_name, result) in tool_results {
                current_messages.push(Message {
                    role: "user".to_string(),
                    content: format!("Tool '{}' returned: {}", tool_name, result),
                });
            }

            debug!("Tool execution iteration {} complete, continuing...", iteration);
        }

        // Return final response (should not reach here in normal flow)
        self.generate_with_tools(&current_messages, system_prompt, false).await
    }

    /// Convert our tool registry to llm crate format
    async fn convert_tools_for_llm(&self) -> Result<Vec<Tool>> {
        let tools_info = self.tool_registry.get_tools_for_ai().await;
        let mut llm_tools = Vec::new();

        for tool_info in tools_info {
            // Convert parameters to llm crate format
            let mut properties = HashMap::new();
            let mut required = Vec::new();

            for param in &tool_info.parameters {
                properties.insert(
                    param.name.clone(),
                    ParameterProperty {
                        param_type: param.param_type.clone(),
                        description: Some(param.description.clone()),
                        enum_values: None,
                    },
                );

                if param.required {
                    required.push(param.name.clone());
                }
            }

            let function = FunctionTool {
                name: tool_info.name.clone(),
                description: Some(tool_info.description.clone()),
                parameters: ParametersSchema {
                    schema_type: "object".to_string(),
                    properties,
                    required,
                },
            };

            llm_tools.push(Tool {
                tool_type: "function".to_string(),
                function,
            });
        }

        debug!("Converted {} tools for LLM", llm_tools.len());
        Ok(llm_tools)
    }
}
