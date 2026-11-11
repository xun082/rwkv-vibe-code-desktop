use serde_json::{json, Value};

// Verbatim webpage prompt from rwkv-vibe-code's Chinese aiService.promptPrefix.
pub const WEBPAGE_PREFIX: &str = r#"用 HTML 和 Tailwind CSS CDN 写一个完整精美、可直接打开的单页网站。页面从上到下依次排列：顶部导航、带大标题和真实文案的主视觉区、三个以上内容区块（图文介绍 / 特色卡片 / 数据或列表）、页脚。每个区块都写满具体文案，并用 Unsplash 真实照片配图，排版精致，配色统一。输出只有代码，从 <!DOCTYPE html> 第一行开始，一路写到 </html> 收尾。需求："#;

// Transport prefill only: no headline, copy, palette, layout or content is supplied.
// Leave body attributes open so the model can design global colors and typography.
pub const HTML_PREFILL: &str = r#"<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0"><script src="https://cdn.tailwindcss.com"></script></head>
<body"#;

pub const STREAMING_INSTRUCTION: &str = r#"你正在流式生成完整精美的HTML网站，用户的全部设计与内容要求均需实现。HTML基础骨架已给出，从<body的属性开始续写。先输出导航品牌与真实主视觉标题，用Tailwind类随正文实现精致的配色、排版、留白与响应式布局。主视觉之后展开图文介绍、特色内容、数据或列表等多个完整区块，分别使用独立section，每个区块有标题、具体文案和配图，并包含用户指定数量的卡片及所有模块，最后完成页脚。额外自定义CSS和JavaScript放在正文之后、body结束之前。仅续写HTML代码，完整写到</body></html>。"#;

// Apply the model's original template with an open assistant generation turn,
// then append our empty HTML shell. This avoids duplicate reasoning prefixes
// introduced by combining Jinja and assistant-message continuation handling.
pub fn template_request() -> Value {
    json!({"messages":[
        {"role":"system", "content":STREAMING_INSTRUCTION},
        {"role":"user", "content":TEMPLATE_MARKER}
    ], "add_generation_prompt":true, "chat_template_kwargs":{"enable_thinking":false}})
}

pub fn user_prompt(description: &str, direction: &str) -> String {
    let requirement = if direction.trim().is_empty() {
        description.to_owned()
    } else {
        format!("{description}\n{direction}")
    };
    format!("{WEBPAGE_PREFIX} {requirement}")
}

pub const TEMPLATE_MARKER: &str = "__LOCALPAGE_BATCH_REQUEST__";

#[derive(Clone)]
pub struct ChatTemplate {
    prefix: String,
    suffix: String,
}
impl ChatTemplate {
    pub fn from_rendered(rendered: &str) -> Result<Self, String> {
        if rendered.matches(TEMPLATE_MARKER).count() != 1 || !rendered.ends_with(HTML_PREFILL) {
            return Err("模型对话模板不支持 HTML 续写".into());
        }
        let (prefix, suffix) = rendered.split_once(TEMPLATE_MARKER).unwrap();
        Ok(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
        })
    }
    pub fn render(&self, description: &str) -> String {
        format!(
            "{}{}{}",
            self.prefix,
            user_prompt(description, ""),
            self.suffix
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_preserves_the_full_request_and_reference_prompt() {
        let description = "六款饮品、价格、品牌故事、活动、地址和营业时间";
        let user = user_prompt(description, "米白和木棕");
        assert!(user.starts_with(WEBPAGE_PREFIX));
        assert!(user.contains(description));
        assert!(user.ends_with("米白和木棕"));
        let request = template_request();
        assert_eq!(request["messages"][0]["content"], STREAMING_INSTRUCTION);
        assert_eq!(request["messages"][1]["content"], TEMPLATE_MARKER);
        assert_eq!(request["chat_template_kwargs"]["enable_thinking"], false);
        assert!(HTML_PREFILL.ends_with("<body"));
        assert!(!HTML_PREFILL.contains("<h1"));
    }

    #[test]
    fn batch_template_keeps_each_request_separate_and_preserves_model_role_tokens() {
        let template = ChatTemplate::from_rendered(&format!(
            "<s>User: {TEMPLATE_MARKER}\n\nAssistant: <think></think>{HTML_PREFILL}"
        ))
        .unwrap();
        let coffee = template.render("咖啡馆：六款饮品和营业时间");
        let hotel = template.render("酒店：四种客房和预订信息");
        assert!(coffee.starts_with("<s>User: "));
        assert!(coffee.contains(WEBPAGE_PREFIX));
        assert!(coffee.contains("六款饮品"));
        assert!(!coffee.contains("四种客房"));
        assert!(hotel.contains("四种客房"));
        assert!(hotel.ends_with(HTML_PREFILL));
        assert!(ChatTemplate::from_rendered("no marker").is_err());
    }
}
