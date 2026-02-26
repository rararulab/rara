use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContentPart,
    ImageUrlArgs,
};
use rara_sessions::types::{ChatMessage, ContentBlock, MessageContent, MessageRole};

/// Convert a session ChatMessage to an async_openai
/// ChatCompletionRequestMessage.
pub fn to_chat_message(msg: &ChatMessage) -> ChatCompletionRequestMessage {
    match msg.role {
        MessageRole::System => ChatCompletionRequestSystemMessageArgs::default()
            .content(msg.content.as_text())
            .build()
            .expect("system message build should not fail")
            .into(),
        MessageRole::User => match &msg.content {
            MessageContent::Text(text) => ChatCompletionRequestUserMessageArgs::default()
                .content(text.as_str())
                .build()
                .expect("user message build should not fail")
                .into(),
            MessageContent::Multimodal(blocks) => {
                let parts: Vec<ChatCompletionRequestUserMessageContentPart> = blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => {
                            ChatCompletionRequestUserMessageContentPart::Text(
                                ChatCompletionRequestMessageContentPartText { text: text.clone() },
                            )
                        }
                        ContentBlock::ImageUrl { url } => {
                            let image_url = ImageUrlArgs::default()
                                .url(url.as_str())
                                .build()
                                .expect("image URL build should not fail");
                            ChatCompletionRequestUserMessageContentPart::ImageUrl(
                                ChatCompletionRequestMessageContentPartImage { image_url },
                            )
                        }
                    })
                    .collect();
                ChatCompletionRequestUserMessageArgs::default()
                    .content(parts)
                    .build()
                    .expect("multimodal user message build should not fail")
                    .into()
            }
        },
        MessageRole::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
            .content(msg.content.as_text())
            .build()
            .expect("assistant message build should not fail")
            .into(),
        MessageRole::Tool | MessageRole::ToolResult => {
            let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
            ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id)
                .content(msg.content.as_text())
                .build()
                .expect("tool message build should not fail")
                .into()
        }
    }
}

/// Rough token estimate: ~3 chars per token.
pub fn estimate_tokens(text: &str) -> usize { text.chars().count().div_ceil(3) }

/// Estimate total tokens for a message history.
pub fn estimate_history_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens(&m.content.as_text()) + 4)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_chat_message_user() {
        let msg = ChatMessage::user("hello");
        let converted = to_chat_message(&msg);
        assert!(matches!(converted, ChatCompletionRequestMessage::User(_)));
    }

    #[test]
    fn to_chat_message_assistant() {
        let msg = ChatMessage::assistant("response");
        let converted = to_chat_message(&msg);
        assert!(matches!(
            converted,
            ChatCompletionRequestMessage::Assistant(_)
        ));
    }

    #[test]
    fn to_chat_message_system() {
        let msg = ChatMessage::system("you are helpful");
        let converted = to_chat_message(&msg);
        assert!(matches!(converted, ChatCompletionRequestMessage::System(_)));
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello"), 2);
        assert_eq!(estimate_tokens(""), 0);
        let long = "a".repeat(300);
        assert_eq!(estimate_tokens(&long), 100);
    }

    #[test]
    fn estimate_history_tokens_sums() {
        let messages = vec![ChatMessage::user("hello"), ChatMessage::assistant("world!")];
        assert_eq!(estimate_history_tokens(&messages), 12);
    }
}
