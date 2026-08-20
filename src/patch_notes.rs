#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchNote {
    pub title: String,
    pub version: Option<String>,
    pub date: Option<String>,
    pub body_markup: String,
}

struct Heading {
    start: usize,
    end: usize,
    title: String,
}

pub fn parse(input: &str) -> Vec<PatchNote> {
    let headings = headings(input);
    if headings.is_empty() {
        return (!input.trim().is_empty())
            .then(|| PatchNote {
                title: "Patch Notes".to_owned(),
                version: None,
                date: None,
                body_markup: html_to_markup(input),
            })
            .into_iter()
            .collect();
    }
    let entries = headings
        .iter()
        .enumerate()
        .filter(|(index, heading)| {
            *index == 0 || preceded_by_rule(input, heading.start) || release_heading(&heading.title)
        })
        .map(|(_, heading)| heading)
        .collect::<Vec<_>>();

    entries
        .iter()
        .enumerate()
        .map(|(index, heading)| {
            let body_end = entries
                .get(index + 1)
                .map_or(input.len(), |next| next.start);
            let body = strip_trailing_rule(&input[heading.end..body_end]);
            PatchNote {
                title: heading.title.clone(),
                version: version_from_body(body).or_else(|| version_from_heading(&heading.title)),
                date: date_from_heading(&heading.title),
                body_markup: html_to_markup(body),
            }
        })
        .collect()
}

fn headings(input: &str) -> Vec<Heading> {
    let lower = input.to_ascii_lowercase();
    let mut headings = Vec::new();
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find("<h4") {
        let start = offset + relative;
        let after_name = lower.as_bytes().get(start + 3).copied();
        if !matches!(
            after_name,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n')
        ) {
            offset = start + 3;
            continue;
        }
        let Some(open_end) = lower[start..].find('>').map(|value| start + value + 1) else {
            break;
        };
        let Some(close_start) = lower[open_end..]
            .find("</h4>")
            .map(|value| open_end + value)
        else {
            break;
        };
        headings.push(Heading {
            start,
            end: close_start + "</h4>".len(),
            title: plain_text(&input[open_end..close_start]),
        });
        offset = close_start + "</h4>".len();
    }
    headings
}

fn preceded_by_rule(input: &str, heading_start: usize) -> bool {
    let prefix = input[..heading_start].trim_end().to_ascii_lowercase();
    let Some(rule_start) = prefix.rfind("<hr") else {
        return false;
    };
    prefix[rule_start..]
        .find('>')
        .is_some_and(|end| prefix[rule_start + end + 1..].trim().is_empty())
}

fn strip_trailing_rule(input: &str) -> &str {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    let Some(rule_start) = lower.rfind("<hr") else {
        return trimmed;
    };
    if lower[rule_start..]
        .find('>')
        .is_some_and(|end| lower[rule_start + end + 1..].trim().is_empty())
    {
        trimmed[..rule_start].trim_end()
    } else {
        trimmed
    }
}

fn release_heading(title: &str) -> bool {
    let lower = title.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "added"
            | "fixed"
            | "updated"
            | "renderer:"
            | "changelog (w/ spoilers):"
            | "new features and major changes"
            | "minor changes and fixes"
    ) {
        return false;
    }
    date_from_heading(title).is_some()
        || version_from_heading(title).is_some()
        || [
            "internal update",
            "installer update",
            "windows update",
            "windows version update",
            "mac version update",
            "language support update",
            "bugfix",
            "changelog",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn version_from_body(body: &str) -> Option<String> {
    let text = plain_text(body);
    let lower = text.to_ascii_lowercase();
    let marker = "version number:";
    let start = lower.find(marker)? + marker.len();
    version_token(text[start..].split_whitespace().next()?)
}

fn version_from_heading(title: &str) -> Option<String> {
    let words = title.split_whitespace().collect::<Vec<_>>();
    for (index, word) in words.iter().enumerate() {
        let clean = word.trim_matches(|character: char| {
            matches!(character, '(' | ')' | '[' | ']' | ',' | ':' | ';')
        });
        if clean.is_empty() || looks_like_date(clean) {
            continue;
        }
        let lower = clean.to_ascii_lowercase();
        let previous = index
            .checked_sub(1)
            .and_then(|previous| words.get(previous))
            .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphabetic()))
            .unwrap_or("");
        let keyword_number = matches!(
            previous.to_ascii_lowercase().as_str(),
            "patch" | "update" | "version" | "build"
        );
        let starts_like_version = lower
            .strip_prefix(['v', 'b', '#'])
            .is_some_and(|value| value.starts_with(|character: char| character.is_ascii_digit()));
        let dotted_number =
            lower.starts_with(|character: char| character.is_ascii_digit()) && lower.contains('.');
        let plain_keyword_number =
            keyword_number && lower.starts_with(|character: char| character.is_ascii_digit());
        if starts_like_version || dotted_number || plain_keyword_number {
            return version_token(clean);
        }
    }
    None
}

fn version_token(token: &str) -> Option<String> {
    let value = token.trim_matches(|character: char| {
        matches!(character, '(' | ')' | '[' | ']' | ',' | ':' | ';')
    });
    value
        .chars()
        .any(|character| character.is_ascii_digit())
        .then(|| value.to_owned())
}

fn date_from_heading(title: &str) -> Option<String> {
    let mut remainder = title;
    while let Some(open) = remainder.find('(') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find(')') else {
            break;
        };
        let candidate = after_open[..close].trim();
        if looks_like_date(candidate) {
            return Some(candidate.to_owned());
        }
        remainder = &after_open[close + 1..];
    }

    let words = title.split_whitespace().collect::<Vec<_>>();
    for (index, word) in words.iter().enumerate() {
        if !is_month(word) {
            continue;
        }
        let mut start = index;
        if index > 0 && date_number(words[index - 1]) {
            start -= 1;
        }
        let mut end = index + 1;
        while end < words.len() && end <= index + 2 && date_number(words[end]) {
            end += 1;
        }
        return Some(
            words[start..end]
                .join(" ")
                .trim_matches(|character: char| matches!(character, '(' | ')' | '-' | ','))
                .to_owned(),
        );
    }
    None
}

fn looks_like_date(value: &str) -> bool {
    let words = value.split_whitespace().collect::<Vec<_>>();
    if words.iter().any(|word| is_month(word)) && words.iter().any(|word| date_number(word)) {
        return true;
    }
    let clean = value.trim_matches(|character: char| matches!(character, '(' | ')' | ','));
    let separator = if clean.contains('-') {
        '-'
    } else if clean.contains('/') {
        '/'
    } else {
        return false;
    };
    let parts = clean.split(separator).collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit())
        })
}

fn is_month(word: &str) -> bool {
    let clean = word
        .trim_matches(|character: char| !character.is_ascii_alphabetic())
        .to_ascii_lowercase();
    [
        "jan",
        "january",
        "feb",
        "february",
        "mar",
        "march",
        "apr",
        "april",
        "may",
        "jun",
        "june",
        "jul",
        "july",
        "aug",
        "august",
        "sep",
        "sept",
        "september",
        "oct",
        "october",
        "nov",
        "november",
        "dec",
        "december",
    ]
    .contains(&clean.as_str())
}

fn date_number(word: &str) -> bool {
    let clean = word.trim_matches(|character: char| matches!(character, '(' | ')' | ','));
    let digits = clean.trim_end_matches(|character: char| character.is_ascii_alphabetic());
    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
}

fn plain_text(input: &str) -> String {
    let mut text = String::new();
    let mut offset = 0;
    while offset < input.len() {
        if input.as_bytes()[offset] == b'<'
            && let Some(end) = input[offset..].find('>')
        {
            offset += end + 1;
            continue;
        }
        let next = input[offset..]
            .find('<')
            .map_or(input.len(), |value| offset + value);
        text.push_str(&decode_entities(&input[offset..next]));
        offset = next;
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_to_markup(input: &str) -> String {
    let mut output = String::new();
    let mut styles = Vec::new();
    let mut lists = Vec::<(bool, usize)>::new();
    let mut offset = 0;
    while offset < input.len() {
        if input.as_bytes()[offset] == b'<'
            && let Some(relative_end) = input[offset..].find('>')
        {
            let end = offset + relative_end;
            handle_tag(
                &input[offset + 1..end],
                &mut output,
                &mut styles,
                &mut lists,
            );
            offset = end + 1;
            continue;
        }
        let next = input[offset..]
            .find('<')
            .map_or(input.len(), |value| offset + value);
        push_markdown_text(&mut output, &decode_entities(&input[offset..next]));
        offset = next;
    }
    while let Some(style) = styles.pop() {
        output.push_str(style.close());
    }
    output.trim_matches('\n').to_owned()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Bold,
    Italic,
    Mono,
    Large,
    ExtraLarge,
    Link,
}

impl Style {
    const fn close(self) -> &'static str {
        match self {
            Self::Bold => "</b>",
            Self::Italic => "</i>",
            Self::Mono => "</tt>",
            Self::Large | Self::ExtraLarge => "</span>",
            Self::Link => "</span>",
        }
    }
}

fn handle_tag(
    raw: &str,
    output: &mut String,
    styles: &mut Vec<Style>,
    lists: &mut Vec<(bool, usize)>,
) {
    let raw = raw.trim();
    let closing = raw.starts_with('/');
    let name = raw
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    if closing {
        match name.as_str() {
            "strong" | "b" => close_style(output, styles, Style::Bold),
            "em" | "i" => close_style(output, styles, Style::Italic),
            "code" | "pre" => close_style(output, styles, Style::Mono),
            "h2" | "h3" => {
                close_style(output, styles, Style::ExtraLarge);
                push_newline(output);
            }
            "h4" | "h5" => {
                close_style(output, styles, Style::Large);
                push_newline(output);
            }
            "a" => close_style(output, styles, Style::Link),
            "p" | "div" | "li" => push_newline(output),
            "ul" | "ol" => {
                lists.pop();
                push_newline(output);
            }
            _ => {}
        }
        return;
    }

    match name.as_str() {
        "strong" | "b" => open_style(output, styles, Style::Bold, "<b>"),
        "em" | "i" => open_style(output, styles, Style::Italic, "<i>"),
        "code" | "pre" => {
            push_newline(output);
            open_style(output, styles, Style::Mono, "<tt>");
        }
        "h2" | "h3" => {
            push_newline(output);
            open_style(
                output,
                styles,
                Style::ExtraLarge,
                "<span size=\"x-large\" weight=\"bold\">",
            );
        }
        "h4" | "h5" => {
            push_newline(output);
            open_style(
                output,
                styles,
                Style::Large,
                "<span size=\"large\" weight=\"bold\">",
            );
        }
        "a" => open_style(output, styles, Style::Link, "<span underline=\"single\">"),
        "p" | "div" | "br" | "hr" => push_newline(output),
        "ul" => lists.push((false, 0)),
        "ol" => lists.push((true, 0)),
        "li" => {
            push_newline(output);
            if let Some((ordered, index)) = lists.last_mut() {
                *index += 1;
                if *ordered {
                    output.push_str(&format!("{index}. "));
                } else {
                    output.push_str("• ");
                }
            } else {
                output.push_str("• ");
            }
        }
        _ => {}
    }
}

fn open_style(output: &mut String, styles: &mut Vec<Style>, style: Style, markup: &str) {
    output.push_str(markup);
    styles.push(style);
}

fn close_style(output: &mut String, styles: &mut Vec<Style>, style: Style) {
    if styles.last() == Some(&style) {
        output.push_str(style.close());
        styles.pop();
    }
}

fn push_markdown_text(output: &mut String, text: &str) {
    let parts = text.split("**").collect::<Vec<_>>();
    if parts.len() < 3 || parts.len().is_multiple_of(2) {
        output.push_str(&escape_markup(text));
        return;
    }
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            output.push_str(if index % 2 == 1 { "<b>" } else { "</b>" });
        }
        output.push_str(&escape_markup(part));
    }
}

fn push_newline(output: &mut String) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn escape_markup(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_rules_and_release_like_headings_but_keeps_subheadings() {
        let notes = parse(
            "<h4>Patch 2.0 (March 2nd, 2024)</h4><h4>Fixed</h4><ul><li>A</li></ul>\
             <hr><h4>Update 1.9</h4><p>B</p><h4>Patch 1.8</h4><p>C</p>",
        );
        assert_eq!(
            notes
                .iter()
                .map(|note| note.title.as_str())
                .collect::<Vec<_>>(),
            ["Patch 2.0 (March 2nd, 2024)", "Update 1.9", "Patch 1.8"]
        );
        assert!(notes[0].body_markup.contains("Fixed"));
    }

    #[test]
    fn extracts_only_dates_explicitly_present_in_headings() {
        let notes = parse(
            "<h4>Patch #1 - Aug 25th 2023</h4><p>Version Number: 4.1.1.3669438</p>\
             <hr><h4>Version 1.405</h4><p>No date supplied.</p>\
             <hr><h4>1.2.1.4 Hotfix 1 (2024-12-20)</h4>",
        );
        assert_eq!(notes[0].version.as_deref(), Some("4.1.1.3669438"));
        assert_eq!(notes[0].date.as_deref(), Some("Aug 25th 2023"));
        assert_eq!(notes[1].version.as_deref(), Some("1.405"));
        assert_eq!(notes[1].date, None);
        assert_eq!(notes[2].date.as_deref(), Some("2024-12-20"));
    }

    #[test]
    fn preserves_safe_rich_text_and_embedded_markdown() {
        let notes = parse(
            "<h4>Update 1.0</h4><h5>Changes</h5><ol><li><strong>Bold</strong></li>\
             <li>**Also bold** &amp; safe</li></ol><p><a href=\"https://example.com\">Details</a></p>",
        );
        let markup = &notes[0].body_markup;
        assert!(markup.contains("<span size=\"large\" weight=\"bold\">Changes</span>"));
        assert!(markup.contains("1. <b>Bold</b>"));
        assert!(markup.contains("2. <b>Also bold</b> &amp; safe"));
        assert!(markup.contains("<span underline=\"single\">Details</span>"));
    }

    #[test]
    fn keeps_gog_order_instead_of_sorting_by_detected_dates() {
        let notes = parse(
            "<h4>Update 2.0</h4><p>Newest according to GOG.</p>\
             <hr><h4>Update 1.0 (1 January 2030)</h4><p>Older according to GOG.</p>",
        );
        assert_eq!(notes[0].title, "Update 2.0");
        assert_eq!(notes[0].date, None);
    }

    #[test]
    fn keeps_unknown_nonempty_formats_as_one_entry() {
        let notes = parse("<p><strong>Changes</strong></p><ul><li>Fixed a crash.</li></ul>");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Patch Notes");
        assert!(notes[0].body_markup.contains("<b>Changes</b>"));
    }
}
