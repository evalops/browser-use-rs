use browser_use_llm::{
    AnthropicChatModel, ChatModel, GeminiChatModel, OllamaChatModel, OpenAiCompatibleChatModel,
    OpenAiSchemaTransform, OpenAiStructuredOutputMode,
};

use crate::LlmProvider;

pub(crate) fn configured_chat_model(
    provider: LlmProvider,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    structured_output_mode_override: Option<OpenAiStructuredOutputMode>,
) -> anyhow::Result<Box<dyn ChatModel>> {
    match provider {
        LlmProvider::OpenAiCompatible
        | LlmProvider::DeepSeek
        | LlmProvider::Groq
        | LlmProvider::Cerebras
        | LlmProvider::Mistral
        | LlmProvider::OpenRouter
        | LlmProvider::Vercel => configured_openai_wire_chat_model(
            openai_wire_provider_config(provider),
            api_key,
            model,
            base_url,
            structured_output_mode_override,
        ),
        LlmProvider::Anthropic => {
            let api_key = api_key
                .or_else(|| nonempty_env("ANTHROPIC_API_KEY"))
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY or --api-key is required"))?;
            let model = model
                .or_else(|| nonempty_env("ANTHROPIC_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("ANTHROPIC_BASE_URL"))
                .unwrap_or_else(|| "https://api.anthropic.com/v1".to_owned());
            let mut llm = AnthropicChatModel::new(api_key, model).with_base_url(base_url);
            if let Some(version) = nonempty_env("ANTHROPIC_VERSION") {
                llm = llm.with_anthropic_version(version);
            }
            if let Some(max_tokens) = nonempty_env("ANTHROPIC_MAX_TOKENS") {
                llm = llm.with_max_tokens(max_tokens.parse()?);
            }
            Ok(Box::new(llm))
        }
        LlmProvider::Gemini => {
            let api_key = api_key
                .or_else(|| nonempty_env("GEMINI_API_KEY"))
                .ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY or --api-key is required"))?;
            let model = model
                .or_else(|| nonempty_env("GEMINI_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("GEMINI_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("GEMINI_BASE_URL"))
                .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_owned());
            Ok(Box::new(
                GeminiChatModel::new(api_key, model).with_base_url(base_url),
            ))
        }
        LlmProvider::Ollama => {
            let model = model
                .or_else(|| nonempty_env("OLLAMA_MODEL"))
                .ok_or_else(|| anyhow::anyhow!("OLLAMA_MODEL or --model is required"))?;
            let base_url = base_url
                .or_else(|| nonempty_env("OLLAMA_BASE_URL"))
                .or_else(|| nonempty_env("OLLAMA_HOST"))
                .unwrap_or_else(|| "http://localhost:11434".to_owned());
            Ok(Box::new(
                OllamaChatModel::new(model).with_base_url(base_url),
            ))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenAiWireProviderConfig {
    pub(crate) provider_name: &'static str,
    pub(crate) api_key_env: &'static [&'static str],
    pub(crate) model_env: &'static [&'static str],
    pub(crate) base_url_env: &'static [&'static str],
    pub(crate) default_headers: &'static [OpenAiWireDefaultHeader],
    pub(crate) default_model: Option<&'static str>,
    pub(crate) default_base_url: &'static str,
    pub(crate) structured_output_mode: OpenAiStructuredOutputMode,
    pub(crate) schema_transform: OpenAiSchemaTransform,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenAiWireDefaultHeader {
    pub(crate) name: &'static str,
    pub(crate) value_env: &'static [&'static str],
}

fn configured_openai_wire_chat_model(
    config: OpenAiWireProviderConfig,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    structured_output_mode_override: Option<OpenAiStructuredOutputMode>,
) -> anyhow::Result<Box<dyn ChatModel>> {
    let api_key = api_key
        .or_else(|| first_nonempty_env(config.api_key_env))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} or --api-key is required",
                provider_env_list(config.api_key_env)
            )
        })?;
    let model = model
        .or_else(|| first_nonempty_env(config.model_env))
        .or_else(|| config.default_model.map(ToOwned::to_owned))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} or --model is required",
                provider_env_list(config.model_env)
            )
        })?;
    let base_url = base_url
        .or_else(|| first_nonempty_env(config.base_url_env))
        .unwrap_or_else(|| config.default_base_url.to_owned());
    let structured_output_mode =
        default_structured_output_mode(config, &model, structured_output_mode_override);

    let mut llm = OpenAiCompatibleChatModel::new(api_key, model)
        .with_base_url(base_url)
        .with_provider_name(config.provider_name)
        .with_structured_output_mode(structured_output_mode)
        .with_schema_transform(config.schema_transform);
    for (name, value) in openai_wire_default_headers(config, first_nonempty_env) {
        llm = llm.try_with_default_header(name, value)?;
    }

    Ok(Box::new(llm))
}

pub(crate) fn default_structured_output_mode(
    config: OpenAiWireProviderConfig,
    model: &str,
    override_mode: Option<OpenAiStructuredOutputMode>,
) -> OpenAiStructuredOutputMode {
    if let Some(mode) = override_mode {
        return mode;
    }

    match config.provider_name {
        "groq" if model == "moonshotai/kimi-k2-instruct" => OpenAiStructuredOutputMode::ToolCall,
        "vercel" if vercel_prompt_fallback_model(model) => OpenAiStructuredOutputMode::PromptOnly,
        _ => config.structured_output_mode,
    }
}

fn vercel_prompt_fallback_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    lower.starts_with("google/")
        || lower.starts_with("anthropic/")
        || [
            "o1",
            "o3",
            "o4",
            "gpt-oss",
            "gpt-5.2-pro",
            "gpt-5.4-pro",
            "deepseek-r1",
            "-thinking",
            "perplexity/sonar-reasoning",
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
}

pub(crate) fn openai_wire_default_headers<F>(
    config: OpenAiWireProviderConfig,
    lookup: F,
) -> Vec<(&'static str, String)>
where
    F: Fn(&[&str]) -> Option<String>,
{
    config
        .default_headers
        .iter()
        .filter_map(|header| lookup(header.value_env).map(|value| (header.name, value)))
        .collect()
}

pub(crate) fn openai_wire_provider_config(provider: LlmProvider) -> OpenAiWireProviderConfig {
    match provider {
        LlmProvider::OpenAiCompatible => OpenAiWireProviderConfig {
            provider_name: "openai-compatible",
            api_key_env: &["OPENAI_API_KEY"],
            model_env: &["OPENAI_MODEL"],
            base_url_env: &["OPENAI_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://api.openai.com/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::DeepSeek => OpenAiWireProviderConfig {
            provider_name: "deepseek",
            api_key_env: &["DEEPSEEK_API_KEY"],
            model_env: &["DEEPSEEK_MODEL"],
            base_url_env: &["DEEPSEEK_BASE_URL"],
            default_headers: &[],
            default_model: Some("deepseek-chat"),
            default_base_url: "https://api.deepseek.com/v1",
            structured_output_mode: OpenAiStructuredOutputMode::ToolCall,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Groq => OpenAiWireProviderConfig {
            provider_name: "groq",
            api_key_env: &["GROQ_API_KEY"],
            model_env: &["GROQ_MODEL"],
            base_url_env: &["GROQ_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://api.groq.com/openai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Cerebras => OpenAiWireProviderConfig {
            provider_name: "cerebras",
            api_key_env: &["CEREBRAS_API_KEY"],
            model_env: &["CEREBRAS_MODEL"],
            base_url_env: &["CEREBRAS_BASE_URL"],
            default_headers: &[],
            default_model: Some("llama3.1-8b"),
            default_base_url: "https://api.cerebras.ai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::PromptOnly,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Mistral => OpenAiWireProviderConfig {
            provider_name: "mistral",
            api_key_env: &["MISTRAL_API_KEY"],
            model_env: &["MISTRAL_MODEL"],
            base_url_env: &["MISTRAL_BASE_URL"],
            default_headers: &[],
            default_model: Some("mistral-medium-latest"),
            default_base_url: "https://api.mistral.ai/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::MistralCompatible,
        },
        LlmProvider::OpenRouter => OpenAiWireProviderConfig {
            provider_name: "openrouter",
            api_key_env: &["OPENROUTER_API_KEY"],
            model_env: &["OPENROUTER_MODEL"],
            base_url_env: &["OPENROUTER_BASE_URL"],
            default_headers: &[
                OpenAiWireDefaultHeader {
                    name: "HTTP-Referer",
                    value_env: &["OPENROUTER_HTTP_REFERER", "OPENROUTER_APP_URL"],
                },
                OpenAiWireDefaultHeader {
                    name: "X-Title",
                    value_env: &["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"],
                },
                OpenAiWireDefaultHeader {
                    name: "X-OpenRouter-Title",
                    value_env: &["OPENROUTER_X_TITLE", "OPENROUTER_APP_TITLE"],
                },
            ],
            default_model: None,
            default_base_url: "https://openrouter.ai/api/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Vercel => OpenAiWireProviderConfig {
            provider_name: "vercel",
            api_key_env: &["AI_GATEWAY_API_KEY", "VERCEL_OIDC_TOKEN"],
            model_env: &["AI_GATEWAY_MODEL", "VERCEL_MODEL"],
            base_url_env: &["AI_GATEWAY_BASE_URL"],
            default_headers: &[],
            default_model: None,
            default_base_url: "https://ai-gateway.vercel.sh/v1",
            structured_output_mode: OpenAiStructuredOutputMode::JsonSchema,
            schema_transform: OpenAiSchemaTransform::Default,
        },
        LlmProvider::Anthropic | LlmProvider::Gemini | LlmProvider::Ollama => {
            unreachable!("non-OpenAI-wire provider")
        }
    }
}

fn first_nonempty_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| nonempty_env(name))
}

fn provider_env_list(names: &[&str]) -> String {
    names.join(", ")
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}
