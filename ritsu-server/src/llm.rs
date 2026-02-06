//! LLM client with tool calling support using the `llm` crate

use anyhow::{Context, Result};
use futures::StreamExt;
use llm::builder::{FunctionBuilder, LLMBackend, LLMBuilder, ParamBuilder};
use llm::chat::ChatMessage;
use llm::LLMProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::{LlmBackend, LlmConfig, TimeoutConfig};
use crate::tools::ToolRegistry;

/// Message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// LLM response with optional tool calls
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCallInfo>,
}

/// Tool call information
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub name: String,
    pub arguments: HashMap<String, String>,
}

/// Unified LLM client supporting multiple backends
pub struct LlmClient {
    provider: Box<dyn LLMProvider>,
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

        let provider = Self::create_provider(backend, &tool_registry)?;

        Ok(Self {
            provider,
            tool_registry,
        })
    }

    fn create_provider(
        backend: &LlmBackend,
        tool_registry: &Arc<ToolRegistry>,
    ) -> Result<Box<dyn LLMProvider>> {
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
            .and_then(|env_var| std::env::var(env_var).ok())
            .unwrap_or_default();

        // Build LLM with all tools registered
        let tools_info = futures::executor::block_on(tool_registry.get_tools_for_ai());
        
        let mut builder = LLMBuilder::new()
            .backend(provider_type)
            .model(&backend.model)
            .max_tokens(2048)
            .temperature(0.7)
            .timeout_seconds(120); // 2 minutes timeout for streaming (default is 30s)

        if !api_key.is_empty() {
            builder = builder.api_key(api_key);
        }

        // Add all tools from registry
        for tool in tools_info {
            let mut func_builder = FunctionBuilder::new(&tool.name)
                .description(&tool.description);

            let mut required_params = Vec::new();
            for param in tool.parameters {
                func_builder = func_builder.param(
                    ParamBuilder::new(&param.name)
                        .type_of(&param.param_type)
                        .description(&param.description)
                );

                if param.required {
                    required_params.push(param.name);
                }
            }

            if !required_params.is_empty() {
                func_builder = func_builder.required(required_params);
            }

            builder = builder.function(func_builder);
        }

        let provider = builder.build().context("Failed to build LLM provider")?;
        
        Ok(provider)
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
        
        // Add conversation messages
        for (i, msg) in messages.iter().enumerate() {
            let message_builder = match msg.role.as_str() {
                "user" => ChatMessage::user(),
                "assistant" => ChatMessage::assistant(),
                _ => {
                    // System messages become user messages
                    ChatMessage::user()
                }
            };

            // Prepend system prompt to first user message if provided
            let content = if i == 0 {
                system_prompt.as_ref().map_or_else(
                    || msg.content.clone(),
                    |prompt| format!("{prompt}\n\n{}", msg.content)
                )
            } else {
                msg.content.clone()
            };

            chat_messages.push(message_builder.content(&content).build());
        }

        debug!("Sending {} messages to LLM (tools: {})", 
            chat_messages.len(), 
            enable_tools
        );

        // Make the chat request - try with tools first, fallback to without tools if it fails
        let response = if enable_tools {
            match self.provider.chat_with_tools(&chat_messages, self.provider.tools()).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!("Tool calling failed ({}), retrying without tools", e);
                    self.provider.chat(&chat_messages).await
                        .context("Failed to send chat request without tools")?
                }
            }
        } else {
            self.provider.chat(&chat_messages).await
                .context("Failed to send chat request")?
        };

        // Extract content
        let content = response.text().unwrap_or_default();

        // Extract tool calls
        let tool_calls: Vec<ToolCallInfo> = response.tool_calls()
            .map(|calls| {
                calls.iter()
                    .filter_map(|call| {
                        // Parse arguments from JSON string to HashMap
                        let args_map = serde_json::from_str::<HashMap<String, serde_json::Value>>(&call.function.arguments)
                            .ok()?;
                        
                        let args: HashMap<String, String> = args_map.into_iter()
                            .map(|(k, v)| {
                                let value = match v {
                                    serde_json::Value::String(s) => s,
                                    other => other.to_string(),
                                };
                                (k, value)
                            })
                            .collect();

                        Some(ToolCallInfo {
                            name: call.function.name.clone(),
                            arguments: args,
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
    pub async fn execute_tool_calls(&self, tool_calls: &[ToolCallInfo]) -> Vec<(String, String)> {
        let mut results = Vec::new();

        for call in tool_calls {
            info!("Executing tool: {} with args: {:?}", call.name, call.arguments);

            // Execute the tool
            match self.tool_registry.execute(&call.name, call.arguments.clone()).await {
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
                    let error_str = format!("Tool execution failed: {e}");
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
        let mut last_response = None;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                warn!("Reached max iterations ({}) for tool execution loop, returning last response", max_iterations);
                
                // If we have a last response with content, return it
                if let Some(resp) = last_response {
                    return Ok(resp);
                }
                
                // Otherwise generate one final response without tools
                return self.generate_with_tools(&current_messages, system_prompt, false).await;
            }

            // Generate response with tools
            let response = self.generate_with_tools(&current_messages, system_prompt, true).await?;

            // Store this response in case we need it
            last_response = Some(response.clone());

            // If no tool calls, we're done
            if response.tool_calls.is_empty() {
                return Ok(response);
            }

            // Execute tool calls
            let tool_results = self.execute_tool_calls(&response.tool_calls).await;
            
            // Check if all tools failed - if so, don't loop again
            let all_failed = tool_results.iter().all(|(_, result)| result.starts_with("Error:"));
            
            if all_failed && iteration > 2 {
                warn!("All tools failed for {} iterations, generating final response without tools", iteration);
                // Add a message explaining the tool failures
                current_messages.push(Message {
                    role: "user".to_string(),
                    content: "The tools are not working correctly. Please respond without using tools.".to_string(),
                });
                return self.generate_with_tools(&current_messages, system_prompt, false).await;
            }

            // Add assistant message with tool calls (if it has content)
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
                    content: format!("Tool '{tool_name}' returned: {result}"),
                });
            }

            debug!("Tool execution iteration {} complete, continuing...", iteration);
        }
    }

    /// Generate streaming response (no tool execution)
    /// Returns a channel receiver that yields text chunks as they arrive
    pub async fn generate_streaming(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        // Convert our messages to llm crate format
        let mut chat_messages = Vec::new();
        
        // Add conversation messages
        for (i, msg) in messages.iter().enumerate() {
            let message_builder = match msg.role.as_str() {
                "assistant" => ChatMessage::assistant(),
                _ => ChatMessage::user(),
            };

            // Prepend system prompt to first user message if provided
            let content = if i == 0 {
                system_prompt.as_ref().map_or_else(
                    || msg.content.clone(),
                    |prompt| format!("{prompt}\n\n{}", msg.content)
                )
            } else {
                msg.content.clone()
            };

            chat_messages.push(message_builder.content(&content).build());
        }

        debug!("Starting streaming response for {} messages", chat_messages.len());

        // Create a channel for streaming chunks
        let (tx, rx) = mpsc::channel(32);

        // Get the provider's streaming response
        let mut stream = self.provider.chat_stream(&chat_messages).await
            .context("Failed to start streaming chat with LLM provider")?;

        // Spawn a task to forward stream items to the channel
        tokio::spawn(async move {
            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(text) => {
                        if tx.send(Ok(text)).await.is_err() {
                            // Receiver dropped, stop streaming
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Streaming error: {e}"))).await;
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}
