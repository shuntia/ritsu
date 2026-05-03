//! LLM client with tool calling support using the `llm` crate

use anyhow::{Context, Result};
use llm::builder::{FunctionBuilder, LLMBackend, LLMBuilder, ParamBuilder};
use llm::chat::ChatMessage;
use llm::LLMProvider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
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
    backend: LlmBackend,
    timeout_config: TimeoutConfig,
    tool_registry: Arc<ToolRegistry>,
    timeout_seconds: u64,
}

impl LlmClient {
    pub fn new(
        config: &LlmConfig,
        timeout_config: &TimeoutConfig,
        tool_registry: Arc<ToolRegistry>,
    ) -> Result<Self> {
        // Select backend by name (default_backend), falling back to the first configured backend
        let backend = config
            .backends
            .iter()
            .find(|b| b.name == config.default_backend)
            .or_else(|| config.backends.first())
            .context("No LLM backends configured")?
            .clone();

        Ok(Self {
            backend,
            timeout_config: timeout_config.clone(),
            tool_registry,
            timeout_seconds: timeout_config.llm_request_seconds,
        })
    }

    async fn build_provider(
        &self,
        system_prompt: Option<&str>,
        for_chat: bool,
    ) -> Result<Box<dyn LLMProvider>> {
        Self::create_provider(&self.backend, &self.timeout_config, &self.tool_registry, system_prompt, for_chat).await
    }

    async fn create_provider(
        backend: &LlmBackend,
        timeout_config: &TimeoutConfig,
        tool_registry: &Arc<ToolRegistry>,
        system_prompt: Option<&str>,
        for_chat: bool,
    ) -> Result<Box<dyn LLMProvider>> {
        // Detect provider from endpoint
        let provider_type = if backend.endpoint.contains("openai.com")
            || backend.endpoint.contains("api.openai.com")
        {
            LLMBackend::OpenAI
        } else if backend.endpoint.contains("anthropic")
            || backend.endpoint.contains("api.anthropic.com")
        {
            LLMBackend::Anthropic
        } else if backend.endpoint.contains("groq") || backend.endpoint.contains("api.groq.com") {
            LLMBackend::Groq
        } else if backend.endpoint.contains("localhost") || backend.endpoint.contains("ollama") {
            LLMBackend::Ollama
        } else {
            // Default to ollama for custom endpoints
            LLMBackend::Ollama
        };

        info!(
            "Initializing {:?} provider with model: {} (endpoint: {})",
            provider_type, backend.model, backend.endpoint
        );

        // Get API key: prioritize direct key, then environment variable
        let api_key = backend.api_key.clone().or_else(|| {
            backend.api_key_env.as_ref().and_then(|env_var| {
                std::env::var(env_var).ok().or_else(|| {
                    if provider_type != LLMBackend::Ollama {
                        warn!(
                            "API key environment variable '{}' not set for {} backend",
                            env_var, backend.name
                        );
                    }
                    None
                })
            })
        });

        // Warn if no API key for non-Ollama backends
        if api_key.is_none() && provider_type != LLMBackend::Ollama {
            warn!(
                "No API key configured for {} backend ({})",
                backend.name, backend.endpoint
            );
        }

        // Build LLM with tools appropriate for this context
        let tools_info = tool_registry.get_tools_for_ai(for_chat).await;

        let mut builder = LLMBuilder::new()
            .backend(provider_type)
            .model(&backend.model)
            .base_url(&backend.endpoint)
            .max_tokens(2048)
            .temperature(0.7)
            .timeout_seconds(timeout_config.llm_request_seconds);

        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }

        if let Some(prompt) = system_prompt {
            builder = builder.system(prompt);
        }

        // Add all tools from registry
        for tool in tools_info {
            let mut func_builder = FunctionBuilder::new(&tool.name).description(&tool.description);

            let mut required_params = Vec::new();
            for param in tool.parameters {
                func_builder = func_builder.param(
                    ParamBuilder::new(&param.name)
                        .type_of(&param.param_type)
                        .description(&param.description),
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

    /// Generate response for background (non-interactive) context.
    pub async fn generate(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
    ) -> Result<LlmResponse> {
        self.generate_with_tools(messages, system_prompt, true, false)
            .await
    }

    /// Generate response with control over tool usage.
    /// `for_chat` controls whether interactive-only tools (e.g. `set_title`) are included.
    /// The system prompt is injected via the provider builder on each call so
    /// that dynamic per-request prompts are supported with the upstream API.
    pub async fn generate_with_tools(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        enable_tools: bool,
        for_chat: bool,
    ) -> Result<LlmResponse> {
        let provider = self.build_provider(system_prompt, for_chat).await?;

        // Convert our messages to llm crate format (user/assistant only; system
        // messages are handled by the provider via the builder system prompt)
        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|msg| {
                let builder = match msg.role.as_str() {
                    "assistant" => ChatMessage::assistant(),
                    _ => ChatMessage::user(),
                };
                builder.content(&msg.content).build()
            })
            .collect();

        debug!(
            "Sending {} messages to LLM (tools: {})",
            chat_messages.len(),
            enable_tools
        );

        // Make the chat request - try with tools first, fallback to without tools if it fails
        let response = if enable_tools {
            match provider
                .chat_with_tools(&chat_messages, provider.tools())
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    // Inspect the error for Groq rate-limit hints and wait if present, then retry once
                    let err_str = e.to_string();

                    if err_str.contains("429")
                        || err_str.to_lowercase().contains("too many requests")
                        || err_str.to_lowercase().contains("rate limit")
                        || err_str.to_lowercase().contains("rate_limit_exceeded")
                    {
                        // Try to extract a suggested wait like "Please try again in 20.07s"
                        if let Some(idx) = err_str.find("Please try again in") {
                            let start = idx + "Please try again in".len();
                            if let Some(s_pos) = err_str[start..].find('s') {
                                let num_portion = &err_str[start..start + s_pos];
                                let num_clean: String = num_portion
                                    .chars()
                                    .filter(|c| c.is_ascii_digit() || *c == '.')
                                    .collect();

                                if num_clean.is_empty() {
                                    warn!("Tool calling failed ({}), retrying without tools", err_str);
                                    provider
                                        .chat(&chat_messages)
                                        .await
                                        .with_context(|| {
                                            format!(
                                                "Failed to send chat request without tools (timeout: {}s)",
                                                self.timeout_seconds
                                            )
                                        })?
                                } else if let Ok(parsed) = num_clean.parse::<f64>() {
                                    let wait_secs = parsed.ceil() as u64;
                                    info!("Detected Groq rate limit, waiting {}s before retrying tool call", wait_secs);
                                    tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;

                                    // Retry the tool call once after waiting
                                    match provider
                                        .chat_with_tools(&chat_messages, provider.tools())
                                        .await
                                    {
                                        Ok(resp2) => resp2,
                                        Err(e2) => {
                                            warn!("Tool calling failed after waiting ({}), retrying without tools", e2);
                                            provider.chat(&chat_messages).await.with_context(|| {
                                                format!(
                                                    "Failed to send chat request without tools (timeout: {}s)",
                                                    self.timeout_seconds
                                                )
                                            })?
                                        }
                                    }
                                } else {
                                    warn!("Tool calling failed ({}), retrying without tools", err_str);
                                    provider
                                        .chat(&chat_messages)
                                        .await
                                        .with_context(|| {
                                            format!(
                                                "Failed to send chat request without tools (timeout: {}s)",
                                                self.timeout_seconds
                                            )
                                        })?
                                }
                            } else {
                                warn!("Tool calling failed ({}), retrying without tools", err_str);
                                provider
                                    .chat(&chat_messages)
                                    .await
                                    .with_context(|| {
                                        format!(
                                            "Failed to send chat request without tools (timeout: {}s)",
                                            self.timeout_seconds
                                        )
                                    })?
                            }
                        } else {
                            // No suggested wait, fall back to retrying without tools
                            warn!("Tool calling failed ({}), retrying without tools", err_str);
                            provider
                                .chat(&chat_messages)
                                .await
                                .with_context(|| {
                                    format!(
                                        "Failed to send chat request without tools (timeout: {}s)",
                                        self.timeout_seconds
                                    )
                                })?
                        }
                    } else {
                        warn!("Tool calling failed ({}), retrying without tools", err_str);
                        provider
                            .chat(&chat_messages)
                            .await
                            .with_context(|| {
                                format!(
                                    "Failed to send chat request without tools (timeout: {}s)",
                                    self.timeout_seconds
                                )
                            })?
                    }
                }
            }
        } else {
            provider.chat(&chat_messages).await.with_context(|| {
                format!(
                    "Failed to send chat request to LLM (timeout: {}s)",
                    self.timeout_seconds
                )
            })?
        };

        // Extract content
        let content = response.text().unwrap_or_default();

        // Extract tool calls
        let tool_calls: Vec<ToolCallInfo> = response
            .tool_calls()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| {
                        // Parse arguments from JSON string to HashMap
                        let args_map = serde_json::from_str::<HashMap<String, serde_json::Value>>(
                            &call.function.arguments,
                        )
                        .ok()?;

                        let args: HashMap<String, String> = args_map
                            .into_iter()
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
            for (i, call) in tool_calls.iter().enumerate() {
                let keys: Vec<_> = call.arguments.keys().cloned().collect();
                debug!(index = i, tool = %call.name, arg_keys = ?keys, "Parsed tool call");
            }
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
            info!(
                "Executing tool: {} with args: {:?}",
                call.name, call.arguments
            );

            // Execute the tool
            match self
                .tool_registry
                .execute(&call.name, call.arguments.clone())
                .await
            {
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

    /// Generate response with automatic tool execution loop.
    /// `for_chat` determines whether interactive-only tools are available.
    pub async fn generate_with_tool_execution(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        max_iterations: usize,
        for_chat: bool,
    ) -> Result<LlmResponse> {
        let mut current_messages = messages.to_vec();
        let mut iteration = 0;
        let mut last_response = None;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                warn!(
                    "Reached max iterations ({}) for tool execution loop, returning last response",
                    max_iterations
                );

                // If we have a last response with content, return it
                if let Some(resp) = last_response {
                    return Ok(resp);
                }

                // Otherwise generate one final response without tools
                return self
                    .generate_with_tools(&current_messages, system_prompt, false, for_chat)
                    .await;
            }

            // Generate response with tools
            let response = self
                .generate_with_tools(&current_messages, system_prompt, true, for_chat)
                .await?;

            // Store this response in case we need it
            last_response = Some(response.clone());

            // If no tool calls, we're done
            if response.tool_calls.is_empty() {
                return Ok(response);
            }

            // Execute tool calls
            let tool_results = self.execute_tool_calls(&response.tool_calls).await;

            // Check if all tools failed - if so, don't loop again
            let all_failed = tool_results
                .iter()
                .all(|(_, result)| result.starts_with("Error:"));

            if all_failed && iteration > 2 {
                warn!(
                    "All tools failed for {} iterations, generating final response without tools",
                    iteration
                );
                // Add a message explaining the tool failures
                current_messages.push(Message {
                    role: "user".to_string(),
                    content:
                        "The tools are not working correctly. Please respond without using tools."
                            .to_string(),
                });
                return self
                    .generate_with_tools(&current_messages, system_prompt, false, for_chat)
                    .await;
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

            debug!(
                "Tool execution iteration {} complete, continuing...",
                iteration
            );
        }
    }
}

#[cfg(test)]
fn select_backend(cfg: &crate::config::LlmConfig) -> Option<&crate::config::LlmBackend> {
    cfg.backends
        .iter()
        .find(|b| b.name == cfg.default_backend)
        .or_else(|| cfg.backends.first())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlmBackend, LlmConfig};

    #[test]
    fn default_backend_honored() {
        let cfg = LlmConfig {
            default_backend: "openai".to_string(),
            backends: vec![
                LlmBackend {
                    name: "openai".to_string(),
                    endpoint: "https://api.openai.com/v1".to_string(),
                    model: "gpt-4".to_string(),
                    api_key: None,
                    api_key_env: None,
                },
                LlmBackend {
                    name: "ollama".to_string(),
                    endpoint: "http://localhost:11434".to_string(),
                    model: "llama3.2".to_string(),
                    api_key: None,
                    api_key_env: None,
                },
            ],
            disable_tools: false,
        };
        assert_eq!(select_backend(&cfg).map(|b| b.name.clone()), Some("openai".to_string()));
    }

    #[test]
    fn fallback_to_first() {
        let cfg = LlmConfig {
            default_backend: "nonexistent".to_string(),
            backends: vec![LlmBackend {
                name: "ollama".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                model: "llama3.2".to_string(),
                api_key: None,
                api_key_env: None,
            }],
            disable_tools: false,
        };
        assert_eq!(select_backend(&cfg).map(|b| b.name.clone()), Some("ollama".to_string()));
    }
}
