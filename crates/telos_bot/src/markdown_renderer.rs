use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Converts raw Markdown to Telegram-compatible HTML and extracts image URLs.
/// Telegram supported HTML tags: <b>, <i>, <u>, <s>, <a>, <code>, <pre>
pub fn render_markdown_to_telegram(markdown: &str) -> (String, Vec<String>) {
    let parser = Parser::new(markdown);
    let mut html = String::new();
    let mut images = Vec::new();

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => html.push_str("\n\n"),
                Tag::Heading { .. } => html.push_str("<b>"),
                Tag::BlockQuote => html.push_str("<i>"),
                Tag::CodeBlock(_) => html.push_str("<pre><code>"),
                Tag::List(_) => html.push_str("\n"),
                Tag::Item => html.push_str("• "),
                Tag::Emphasis => html.push_str("<i>"),
                Tag::Strong => html.push_str("<b>"),
                Tag::Strikethrough => html.push_str("<s>"),
                Tag::Link { dest_url, .. } => {
                    html.push_str(&format!("<a href=\"{}\">", dest_url));
                }
                Tag::Image { dest_url, .. } => {
                    images.push(dest_url.to_string());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {}
                TagEnd::Heading { .. } => html.push_str("</b>\n\n"),
                TagEnd::BlockQuote => html.push_str("</i>\n"),
                TagEnd::CodeBlock => html.push_str("</code></pre>\n"),
                TagEnd::List(_) => html.push_str("\n"),
                TagEnd::Item => html.push_str("\n"),
                TagEnd::Emphasis => html.push_str("</i>"),
                TagEnd::Strong => html.push_str("</b>"),
                TagEnd::Strikethrough => html.push_str("</s>"),
                TagEnd::Link => html.push_str("</a>"),
                TagEnd::Image => {}
                _ => {}
            },
            Event::Text(text) => {
                html.push_str(&teloxide::utils::html::escape(&text));
            }
            Event::Code(text) => {
                html.push_str("<code>");
                html.push_str(&teloxide::utils::html::escape(&text));
                html.push_str("</code>");
            }
            Event::Html(text) => {
                html.push_str(&teloxide::utils::html::escape(&text));
            }
            Event::SoftBreak | Event::HardBreak => {
                html.push_str("\n");
            }
            _ => {}
        }
    }

    (html.trim().to_string(), images)
}
