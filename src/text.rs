/// Converts the small HTML fragments returned by GOG into readable plain text.
pub fn html_to_text(input: &str) -> String {
    let mut output = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .trim()
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if matches!(name, "p" | "br" | "li" | "div" | "h1" | "h2" | "h3")
                    && !output.ends_with('\n')
                {
                    output.push('\n');
                }
                tag.clear();
            }
            _ if in_tag => tag.push(character),
            _ => output.push(character),
        }
    }
    let output = output
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_gog_html_fragments() {
        assert_eq!(
            html_to_text("<p>Hello &amp; welcome</p><ul><li>Cloud saves</li><li>Linux</li></ul>"),
            "Hello & welcome\nCloud saves\nLinux"
        );
    }
}
