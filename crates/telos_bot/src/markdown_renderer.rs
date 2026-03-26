use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Converts raw Markdown to Telegram-compatible HTML and extracts image URLs.
/// Telegram supported HTML tags: <b>, <i>, <u>, <s>, <a>, <code>, <pre>
pub fn render_markdown_to_telegram(markdown: &str) -> (String, Vec<String>) {
    let parser = Parser::new(markdown);
    let mut html = String::new();
    let mut images = Vec::new();
    let mut in_table = false;
    let mut table_row_cells: Vec<String> = Vec::new();
    let mut table_is_header = false;
    let mut table_rows: Vec<(bool, Vec<String>)> = Vec::new(); // (is_header, cells)

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    // Only add spacing if there's already content (avoid leading blank lines)
                    if !html.is_empty() && !html.ends_with('\n') {
                        html.push('\n');
                    }
                }
                Tag::Heading { .. } => {
                    // Heading starts on its own line, one blank line above for separation
                    if !html.is_empty() {
                        if !html.ends_with('\n') {
                            html.push('\n');
                        }
                    }
                    html.push_str("<b>");
                }
                Tag::BlockQuote => html.push_str("<i>"),
                Tag::CodeBlock(_) => html.push_str("<pre><code>"),
                Tag::List(_) => {
                    if !html.is_empty() && !html.ends_with('\n') {
                        html.push('\n');
                    }
                }
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
                Tag::Table(_) => {
                    in_table = true;
                    table_rows.clear();
                    if !html.is_empty() && !html.ends_with('\n') {
                        html.push('\n');
                    }
                }
                Tag::TableHead => {
                    table_is_header = true;
                }
                Tag::TableRow => {
                    table_row_cells.clear();
                }
                Tag::TableCell => {
                    table_row_cells.push(String::new());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    html.push('\n');
                }
                TagEnd::Heading { .. } => {
                    // Heading ends: just close tag + single newline (no blank line gap)
                    html.push_str("</b>\n");
                }
                TagEnd::BlockQuote => html.push_str("</i>\n"),
                TagEnd::CodeBlock => html.push_str("</code></pre>\n"),
                TagEnd::List(_) => {
                    // No extra newline — items already end with \n
                }
                TagEnd::Item => html.push('\n'),
                TagEnd::Emphasis => html.push_str("</i>"),
                TagEnd::Strong => html.push_str("</b>"),
                TagEnd::Strikethrough => html.push_str("</s>"),
                TagEnd::Link => html.push_str("</a>"),
                TagEnd::Image => {}
                TagEnd::Table => {
                    in_table = false;
                    // Render table as aligned text using monospace
                    if !table_rows.is_empty() {
                        // Calculate column widths
                        let num_cols = table_rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
                        let mut col_widths = vec![0usize; num_cols];
                        for (_, cells) in &table_rows {
                            for (i, cell) in cells.iter().enumerate() {
                                if i < num_cols {
                                    // Count display width (CJK chars = 2, ASCII = 1)
                                    let w: usize = cell.chars().map(|c| {
                                        if c >= '\u{2E80}' && c <= '\u{9FFF}' || c >= '\u{F900}' && c <= '\u{FAFF}' || c >= '\u{FF00}' && c <= '\u{FFEF}' {
                                            2
                                        } else {
                                            1
                                        }
                                    }).sum();
                                    col_widths[i] = col_widths[i].max(w);
                                }
                            }
                        }

                        for (is_header, cells) in &table_rows {
                            let mut row_str = String::from("│ ");
                            for (i, cell) in cells.iter().enumerate() {
                                let target_w = col_widths.get(i).copied().unwrap_or(0);
                                let cell_w: usize = cell.chars().map(|c| {
                                    if c >= '\u{2E80}' && c <= '\u{9FFF}' || c >= '\u{F900}' && c <= '\u{FAFF}' || c >= '\u{FF00}' && c <= '\u{FFEF}' {
                                        2
                                    } else {
                                        1
                                    }
                                }).sum();
                                let padding = target_w.saturating_sub(cell_w);
                                if *is_header {
                                    row_str.push_str(&format!("<b>{}</b>", teloxide::utils::html::escape(cell)));
                                } else {
                                    row_str.push_str(&teloxide::utils::html::escape(cell));
                                }
                                for _ in 0..padding {
                                    row_str.push(' ');
                                }
                                if i < cells.len() - 1 {
                                    row_str.push_str(" │ ");
                                }
                            }
                            row_str.push_str(" │");
                            html.push_str(&row_str);
                            html.push('\n');

                            // Add separator line after header
                            if *is_header {
                                let mut sep = String::from("├─");
                                for (i, w) in col_widths.iter().enumerate() {
                                    for _ in 0..*w {
                                        sep.push('─');
                                    }
                                    if i < col_widths.len() - 1 {
                                        sep.push_str("─┼─");
                                    }
                                }
                                sep.push_str("─┤");
                                html.push_str(&sep);
                                html.push('\n');
                            }
                        }
                    }
                    table_rows.clear();
                }
                TagEnd::TableHead => {
                    table_is_header = false;
                }
                TagEnd::TableRow => {
                    table_rows.push((table_is_header, table_row_cells.clone()));
                    table_row_cells.clear();
                }
                TagEnd::TableCell => {
                    // Cell text has been accumulated — push it
                    // (handled specially below)
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_table {
                    // In table: accumulate into the current cell
                    if let Some(last) = table_row_cells.last_mut() {
                        last.push_str(&text);
                    } else {
                        table_row_cells.push(text.to_string());
                    }
                } else {
                    html.push_str(&teloxide::utils::html::escape(&text));
                }
            }
            Event::Code(text) => {
                if in_table {
                    if let Some(last) = table_row_cells.last_mut() {
                        last.push_str(&text);
                    } else {
                        table_row_cells.push(text.to_string());
                    }
                } else {
                    html.push_str("<code>");
                    html.push_str(&teloxide::utils::html::escape(&text));
                    html.push_str("</code>");
                }
            }
            Event::Html(text) => {
                html.push_str(&teloxide::utils::html::escape(&text));
            }
            Event::SoftBreak | Event::HardBreak => {
                html.push('\n');
            }
            _ => {}
        }
    }

    // Post-processing: collapse 3+ consecutive newlines into 2
    let mut result = String::with_capacity(html.len());
    let mut consecutive_newlines = 0;
    for ch in html.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                result.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            result.push(ch);
        }
    }

    (result.trim().to_string(), images)
}
