//! Prompt builder utilities for assembling system prompts and injecting runtime context
#![allow(clippy::missing_const_for_fn)]

use anyhow::Result;
use chrono::Local;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::RwLock;
use tracing::warn;

use crate::llm::Message as LlmMessage;
use crate::memory::MemoryManager;
use crate::tasks::TaskManager;

/// Pre-prompt hook type: async function that returns optional injected text
pub type PrePromptHook = Arc<
    dyn Fn() -> Pin<Box<dyn std::future::Future<Output = Result<Option<String>>> + Send>>
        + Send
        + Sync,
>;

static PRE_PROMPT_HOOKS: OnceLock<Arc<RwLock<Vec<PrePromptHook>>>> = OnceLock::new();

fn hooks() -> &'static Arc<RwLock<Vec<PrePromptHook>>> {
    PRE_PROMPT_HOOKS.get_or_init(|| Arc::new(RwLock::new(Vec::new())))
}

/// Register a pre-LLM hook that can inject context into prompts
pub async fn register_pre_hook(hook: PrePromptHook) {
    let h = hooks();
    let mut w = h.write().await;
    w.push(hook);
}

/// Clear registered hooks (useful for tests)
#[cfg(test)]
pub async fn clear_pre_hooks() {
    let h = hooks();
    let mut w = h.write().await;
    w.clear();
}

/// Run all pre-LLM hooks and collect their outputs (non-empty)
pub async fn run_pre_hooks() -> Result<Vec<String>> {
    let h = hooks();
    let hook_list = {
        let r = h.read().await;
        r.clone()
    };

    let mut outputs = Vec::new();
    for hook in hook_list {
        match (hook)().await {
            Ok(Some(s)) if !s.is_empty() => outputs.push(s),
            Ok(_) => {}
            Err(e) => warn!("Pre-prompt hook failed: {}", e),
        }
    }
    Ok(outputs)
}

pub struct PromptBuilder;

impl PromptBuilder {
    /// Build chat system prompt and messages by injecting runtime context (time, tasks)
    pub async fn build_chat(
        memory: &MemoryManager,
        task_manager: &TaskManager,
        mut history: Vec<LlmMessage>,
        user_content: &str,
    ) -> Result<(Option<String>, Vec<LlmMessage>)> {
        // Try to get chat-specific system prompt, fall back to a minimal default
        let system_prompt = match memory.build_chat_prompt().await {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("Failed to build chat prompt: {}", e);
                Some("You are Ritsu, a helpful AI assistant. You are in chat mode - respond directly to the user.".to_string())
            }
        };

        // Run pre-prompt hooks for injected context; fall back to direct task summary if none
        let hook_outputs = match run_pre_hooks().await {
            Ok(h) => h,
            Err(e) => {
                warn!("Pre-prompt hooks failed: {}", e);
                Vec::new()
            }
        };
        let context_block = if !hook_outputs.is_empty() {
            hook_outputs.join("\n\n")
        } else {
            match task_manager.get_task_summary().await {
                Ok(s) => s,
                Err(_) => "Unable to retrieve task summary".to_string(),
            }
        };

        // Inject current local time
        let now = Local::now();
        let time_str = now.format("%A, %B %d, %Y at %I:%M %p").to_string();

        let enhanced_content = format!("[Current Time: {time_str}]\n[{context_block}]\n\n{user_content}");

        history.push(LlmMessage {
            role: "user".to_string(),
            content: enhanced_content,
        });

        Ok((system_prompt, history))
    }

    /// Build background system prompt and a single user message, running pre-hooks
    pub async fn build_background(
        memory: &MemoryManager,
        _task_manager: &TaskManager,
        analysis_type: Option<&str>,
        user_content: Option<&str>,
    ) -> Result<(Option<String>, Vec<LlmMessage>)> {
        // Get background/system prompt for the analysis type
        let system_prompt = match memory.build_background_prompt(analysis_type).await {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("Failed to build background prompt: {}", e);
                None
            }
        };

        // Run pre-prompt hooks
        let hook_outputs = match run_pre_hooks().await {
            Ok(h) => h,
            Err(e) => {
                warn!("Pre-prompt hooks failed: {}", e);
                Vec::new()
            }
        };

        // Compose user message: hooks first, then provided content
        let mut user_text = String::new();
        if !hook_outputs.is_empty() {
            user_text.push_str(&hook_outputs.join("\n\n"));
            if let Some(uc) = user_content {
                user_text.push_str("\n\n");
                user_text.push_str(uc);
            }
        } else if let Some(uc) = user_content {
            // If no hooks, include direct task summary fallback inside user_content where appropriate
            user_text.push_str(uc);
        }

        let messages = vec![LlmMessage {
            role: "user".to_string(),
            content: user_text,
        }];

        Ok((system_prompt, messages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pre_hooks_run_and_clear() {
        // Ensure clean slate
        clear_pre_hooks().await;

        // Register a simple hook that returns a fixed string
        register_pre_hook(Arc::new(|| {
            Box::pin(async move { Ok(Some("TESTHOOK".to_string())) })
        }))
        .await;

        let outputs = run_pre_hooks().await.unwrap();
        assert_eq!(outputs, vec!["TESTHOOK".to_string()]);

        clear_pre_hooks().await;
        let outputs2 = run_pre_hooks().await.unwrap();
        assert!(outputs2.is_empty());
    }
}
