use adw::prelude::*;

pub(in crate::ui) fn text_excerpt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut excerpt = text.chars().take(max_chars).collect::<String>();
    if let Some(boundary) = excerpt.rfind([' ', '\n']) {
        excerpt.truncate(boundary);
    }
    excerpt.push('…');
    excerpt
}

pub(in crate::ui) fn section(title: &str, body: &str) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.add_css_class("section-title");
    let text = gtk::Label::new(Some(body));
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_selectable(true);
    text.add_css_class("body-copy");
    box_.append(&heading);
    box_.append(&text);
    box_
}

pub(in crate::ui) fn expandable_section(title: &str, body: String, max_chars: usize) -> gtk::Box {
    if body.chars().count() <= max_chars {
        return section(title, &body);
    }

    let mut preview = body.chars().take(max_chars).collect::<String>();
    if let Some(last_break) = preview.rfind([' ', '\n']) {
        preview.truncate(last_break);
    }
    preview.push('…');

    let box_ = section(title, &preview);
    let text = box_
        .last_child()
        .and_downcast::<gtk::Label>()
        .expect("section text label");
    let toggle = gtk::Button::with_label("Show more");
    toggle.set_halign(gtk::Align::Start);
    toggle.add_css_class("flat");
    toggle.add_css_class("accent");
    let full_text = body;
    toggle.connect_clicked(move |button| {
        let expanded = button.label().as_deref() == Some("Show less");
        if expanded {
            text.set_label(&preview);
            button.set_label("Show more");
        } else {
            text.set_label(&full_text);
            button.set_label("Show less");
        }
    });
    box_.append(&toggle);
    box_
}

pub(in crate::ui) fn empty_dash(value: &str) -> &str {
    if value.is_empty() { "—" } else { value }
}
