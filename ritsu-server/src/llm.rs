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
    timeout_seconds: u64,
}

impl LlmClient {
    pub async fn new(
        config: &LlmConfig,
        timeout_config: &TimeoutConfig,
        tool_registry: Arc<ToolRegistry>,
    ) -> Result<Self> {
        // Use the first backend for now (can be extended to support multiple)
        let backend = config
            .backends
            .first()
            .context("No LLM backends configured")?;

        let provider = Self::create_provider(backend, timeout_config, &tool_registry).await?;

        Ok(Self {
            provider,
            tool_registry,
            timeout_seconds: timeout_config.llm_request_seconds,
        })
    }

    async fn create_provider(
        backend: &LlmBackend,
        timeout_config: &TimeoutConfig,
        tool_registry: &Arc<ToolRegistry>,
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
                        warn!("API key environment variable '{}' not set for {} backend", 
                            env_var, backend.name);
                    }
                    None
                })
            })
        });

        // Warn if no API key for non-Ollama backends
        if api_key.is_none() && provider_type != LLMBackend::Ollama {
            warn!("No API key configured for {} backend ({})", backend.name, backend.endpoint);
        }

        // Build LLM with all tools registered
        let tools_info = tool_registry.get_tools_for_ai().await;

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

    /// Get tools in llm crate format
    async fn get_llm_tools(&self) -> Vec<llm::chat::Tool> {
        let tools_info = self.tool_registry.get_tools_for_ai().await;

        tools_info
            .iter()
            .map(|tool_info| {
                // Convert parameters to JSON schema
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();

                for param in &tool_info.parameters {
                    // Build property schema
                    let mut prop = serde_json::Map::new();
                    prop.insert("type".to_string(), serde_json::json!(param.param_type));
                    prop.insert(
                        "description".to_string(),
                        serde_json::json!(param.description),
                    );
                    properties.insert(param.name.clone(), serde_json::Value::Object(prop));

                    if param.required {
                        required.push(param.name.clone());
                    }
                }

                let parameters = serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                });

                llm::chat::Tool {
                    tool_type: "function".to_string(),
                    function: llm::chat::FunctionTool {
                        name: tool_info.name.clone(),
                        description: tool_info.description.clone(),
                        parameters,
                    },
                }
            })
            .collect()
    }

    /// Generate response with optional tool calling support
    pub async fn generate(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
    ) -> Result<LlmResponse> {
        self.generate_with_tools(messages, system_prompt, true)
            .await
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

        // If a system prompt is provided, send it as a distinct system message
        if let Some(prompt) = system_prompt {
            chat_messages.push(ChatMessage::system().content(prompt).build());
        }

        // Add conversation messages
        for msg in messages {
            let message_builder = match msg.role.as_str() {
                "assistant" => ChatMessage::assistant(),
                "system" => ChatMessage::system(),
                _ => ChatMessage::user(),
            };

            chat_messages.push(message_builder.content(&msg.content).build());
        }

        debug!(
            "Sending {} messages to LLM (tools: {})",
            chat_messages.len(),
            enable_tools
        );

        // Make the chat request - try with tools first, fallback to without tools if it fails
        let response = if enable_tools {
            match self
                .provider
                .chat_with_tools(&chat_messages, self.provider.tools())
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    warn!("Tool calling failed ({}), retrying without tools", e);
                    self.provider.chat(&chat_messages).await.with_context(|| {
                        format!(
                            "Failed to send chat request without tools (timeout: {}s)",
                            self.timeout_seconds
                        )
                    })?
                }
            }
        } else {
            self.provider.chat(&chat_messages).await.with_context(|| {
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
                    .generate_with_tools(&current_messages, system_prompt, false)
                    .await;
            }

            // Generate response with tools
            let response = self
                .generate_with_tools(&current_messages, system_prompt, true)
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
                    .generate_with_tools(&current_messages, system_prompt, false)
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

    /// Generate streaming response and perform post-stream tool execution + synthesis
    ///
    /// Collects tool calls that appear in the stream, executes them after the
    /// stream completes, and synthesizes a follow-up assistant message using a
    /// non-streaming LLM request. The synthesized follow-up is sent as an
    /// additional chunk on the same channel.
    pub async fn generate_streaming(self: Arc<Self>,
        messages: &[Message],
        system_prompt: Option<&str>,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        // Convert our messages to llm crate format
        let mut chat_messages = Vec::new();

        // If a system prompt is provided, send it as a distinct system message
        if let Some(prompt) = system_prompt {
            chat_messages.push(ChatMessage::system().content(prompt).build());
        }

        // Add conversation messages
        for msg in messages {
            let message_builder = match msg.role.as_str() {
                "assistant" => ChatMessage::assistant(),
                "system" => ChatMessage::system(),
                _ => ChatMessage::user(),
            };

            chat_messages.push(message_builder.content(&msg.content).build());
        }

        debug!(
            "Starting streaming response for {} messages",
            chat_messages.len()
        );

        // Create a channel for streaming chunks
        let (tx, rx) = mpsc::channel(32);

        // Get tools in llm crate format
        let mut tools = self.get_llm_tools().await;
        let tool_count = tools.len();
        debug!("Streaming with {} tools available", tool_count);

        // Limit tools sent during streaming to avoid very large payloads that can
        // slow down or break streaming in some LLM servers (e.g., Ollama).
        const STREAM_TOOL_LIMIT: usize = 10;
        if tool_count > STREAM_TOOL_LIMIT {
            debug!("Tool count {} exceeds streaming threshold; limiting to first {} tools", tool_count, STREAM_TOOL_LIMIT);
            tools.truncate(STREAM_TOOL_LIMIT);
        }

        // Get the provider's streaming response with tools
        let mut stream = self
            .provider
            .chat_stream_with_tools(&chat_messages, Some(&tools))
            .await
            .context("Failed to start streaming chat with LLM provider")?;

        // Clone tool registry for post-processing
        let tool_registry = self.tool_registry.clone();

        // Spawn a task to forward stream items to the channel and collect tool calls
        tokio::spawn(async move {
            use llm::chat::StreamChunk;

            let mut full_response = String::new();
            let mut collected_calls: Vec<ToolCallInfo> = Vec::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        match chunk {
                            StreamChunk::Text(text) => {
                                // Accumulate textual output for later synthesis
                                full_response.push_str(&text);

                                if tx.send(Ok(text)).await.is_err() {
                                    // Receiver dropped, stop streaming
                                    break;
                                }
                            }

                            StreamChunk::ToolUseComplete { index: _, tool_call } => {
                                // Collect tool calls for post-stream execution
                                tracing::debug!(
                                    "Collected tool call in stream: {} with args: {}",
                                    tool_call.function.name,
                                    tool_call.function.arguments
                                );

                                // Parse arguments JSON into HashMap<String, String>
                                let args_map: HashMap<String, serde_json::Value> =
                                    serde_json::from_str(&tool_call.function.arguments).unwrap_or_default();

                                let mut args: HashMap<String, String> = HashMap::new();
                                for (k, v) in args_map.into_iter() {
                                    let val_str = match v {
                                        serde_json::Value::String(s) => s,
                                        other => other.to_string(),
                                    };
                                    args.insert(k, val_str);
                                }

                                collected_calls.push(ToolCallInfo {
                                    name: tool_call.function.name.clone(),
                                    arguments: args,
                                });
                            }

                            StreamChunk::ToolUseStart { index: _, id: _, name } => {
                                tracing::debug!("Tool use started in stream: {}", name);
                            }

                            StreamChunk::ToolUseInputDelta { index: _, partial_json } => {
                                tracing::debug!("Tool input delta in stream: {}", partial_json);
                            }

                            StreamChunk::Done { stop_reason } => {
                                tracing::debug!("Stream done: {}", stop_reason);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Streaming error: {e}"))).await;
                        break;
                    }
                }
            }

            // If tool calls were collected, execute them now using a bounded worker pool with per-tool timeouts
            if !collected_calls.is_empty() {
                tracing::info!("Executing {} collected tool call(s) after stream completion", collected_calls.len());

                // Bounded concurrency for tool execution to avoid resource exhaustion
                let max_concurrent_tools = 4usize;
                let tool_timeout = std::time::Duration::from_secs(30);
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrent_tools));

                // Spawn each tool execution into its own task, limited by the semaphore
                let mut handles = Vec::new();
                for call in collected_calls.into_iter() {
                    let permit_sem = semaphore.clone();
                    let tool_registry = tool_registry.clone();
                    let tx_clone = tx.clone();
                    let call_name = call.name.clone();
                    let call_args = call.arguments.clone();

                    let handle = tokio::spawn(async move {
                        // Acquire a permit (await inside spawned task so we don't block the outer task)
                        let _permit = permit_sem.acquire_owned().await.expect("semaphore closed");

                        // Log start
                        let start = std::time::Instant::now();
                        info!(tool=%call_name, args=?call_args, "Starting tool execution");

                        // Execute with timeout to prevent a single tool from blocking forever
                        match tokio::time::timeout(tool_timeout, tool_registry.execute(&call_name, call_args.clone())).await {
                            Ok(Ok(res)) => {
                                let duration = start.elapsed();
                                let result_text = if res.success { res.output.clone() } else { res.error.clone().unwrap_or_else(|| res.output.clone()) };
                                info!(tool=%call_name, duration_ms = %duration.as_millis(), success = res.success, "Tool execution completed");

                                let msg = format!("\n\n🔧 Tool '{}' result:\n{}\n", call_name, result_text);
                                let _ = tx_clone.send(Ok(msg)).await;
                            }
                            Ok(Err(e)) => {
                                let duration = start.elapsed();
                                warn!(tool=%call_name, duration_ms = %duration.as_millis(), error=%e, "Tool execution failed");

                                let msg = format!("\n\n🔧 Tool '{}' execution failed: {}\n", call_name, e);
                                let _ = tx_clone.send(Ok(msg)).await;
                            }
                            Err(_) => {
                                let duration = start.elapsed();
                                warn!(tool=%call_name, duration_ms = %duration.as_millis(), "Tool execution timed out after {}s", tool_timeout.as_secs());

                                let msg = format!("\n\n🔧 Tool '{}' execution timed out after {}s\n", call_name, tool_timeout.as_secs());
                                let _ = tx_clone.send(Ok(msg)).await;
                            }
                        }
                        // _permit dropped here
                    });

                    handles.push(handle);
                }

                // Wait for all spawned tool tasks to finish and then close the sender
                let _ = futures::future::join_all(handles).await;
            }

            // Drop the sender to close the channel
            drop(tx);
        });

        Ok(rx)
    }
}
