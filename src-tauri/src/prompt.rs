// Verbatim webpage prompt from rwkv-vibe-code's Chinese aiService.promptPrefix.
pub const WEBPAGE_PREFIX: &str = r#"用 HTML 和 Tailwind CSS CDN 写一个完整精美、可直接打开的单页网站。页面从上到下依次排列：顶部导航、带大标题和真实文案的主视觉区、三个以上内容区块（图文介绍 / 特色卡片 / 数据或列表）、页脚。每个区块都写满具体文案，并用 Unsplash 真实照片配图，排版精致，配色统一。输出只有代码，从 <!DOCTYPE html> 第一行开始，一路写到 </html> 收尾。需求："#;

// Transport prefill only: no headline, copy, palette, layout or content is supplied.
// Leave body attributes open so the model can design global colors and typography.
pub const HTML_PREFILL: &str = r#"<!DOCTYPE html>
<html lang="zh-CN"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0"><script src="https://cdn.tailwindcss.com"></script></head>
<body"#;

pub const STREAMING_INSTRUCTION: &str = r#"你正在流式生成完整精美的HTML网站，用户的全部设计与内容要求均需实现。HTML基础骨架已给出，从<body的属性开始续写。先输出导航品牌与真实主视觉标题，用Tailwind类随正文实现精致的配色、排版、留白与响应式布局。主视觉之后展开图文介绍、特色内容、数据或列表等多个完整区块，分别使用独立section，每个区块有标题、具体文案和配图，并包含用户指定数量的卡片及所有模块，最后完成页脚。额外自定义CSS和JavaScript放在正文之后、body结束之前。仅续写HTML代码，完整写到</body></html>。"#;

// RWKV-7 G1l's non-thinking chat template; the native batch API takes raw prompts.
pub fn render(description: &str) -> String {
    format!(
        "System: {STREAMING_INSTRUCTION}\n\nUser: {WEBPAGE_PREFIX} {}\n\nAssistant: <think></think>\n{HTML_PREFILL}",
        description.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_distinct_requirements_with_the_rwkv7_continuation() {
        let coffee = render("咖啡馆：六款饮品和营业时间");
        let hotel = render("酒店：四种客房和预订信息");
        assert!(coffee.starts_with("System: "));
        assert!(coffee.contains(&format!("\n\nUser: {WEBPAGE_PREFIX}")));
        assert!(coffee.contains("六款饮品"));
        assert!(!coffee.contains("四种客房"));
        assert!(hotel.contains("四种客房"));
        assert!(hotel.contains("\n\nAssistant: <think></think>\n<!DOCTYPE html>"));
        assert!(hotel.ends_with(HTML_PREFILL));
    }
}
