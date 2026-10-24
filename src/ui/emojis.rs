use super::*;
use crate::protocol::LineEmoji;

#[derive(Clone)]
pub(super) struct EmojiText {
    view: gtk::TextView,
    text: String,
    emojis: Vec<LineEmoji>,
}

fn ranges(text: &str, emojis: &[LineEmoji]) -> Vec<(usize, usize, String)> {
    let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    boundaries.push(text.len());
    let mut previous = 0;
    emojis
        .iter()
        .take(100)
        .filter_map(|emoji| {
            if emoji.start < previous || emoji.end <= emoji.start {
                return None;
            }
            let (&start, &end) = (boundaries.get(emoji.start)?, boundaries.get(emoji.end)?);
            previous = emoji.end;
            Some((start, end, emoji.key.clone()))
        })
        .collect()
}

fn render(textures: &HashMap<String, gdk::Texture>, content: &EmojiText) {
    let buffer = content.view.buffer();
    buffer.set_text("");
    let mut iter = buffer.end_iter();
    let mut previous = 0;
    for (start, end, key) in ranges(&content.text, &content.emojis) {
        buffer.insert(&mut iter, &content.text[previous..start]);
        if let Some(texture) = textures.get(&key) {
            buffer.insert_paintable(&mut iter, texture);
        } else {
            // 取得失敗時も代替テキストを残し、本文を欠落させない。
            buffer.insert(&mut iter, &content.text[start..end]);
        }
        previous = end;
    }
    buffer.insert(&mut iter, &content.text[previous..]);
    content.view.queue_resize();
}

pub(super) fn append(state: &AppState, bubble: &gtk::Box, message: &MessageInfo) {
    let view: gtk::TextView = glib::Object::builder::<super::emoji_view::EmojiView>()
        .property("editable", false)
        .property("cursor-visible", false)
        .property("wrap-mode", gtk::WrapMode::WordChar)
        .property("accepts-tab", false)
        .build()
        .upcast();
    view.set_css_classes(&["line-bubble-text", "line-emoji-text"]);
    view.set_extra_menu(Some(&replies::menu(message)));
    view.update_property(&[gtk::accessible::Property::Label(&message.text)]);
    let content = EmojiText {
        view: view.clone(),
        text: message.text.clone(),
        emojis: message.emojis.clone(),
    };
    render(&state.emoji_textures.borrow(), &content);
    state.emoji_views.borrow_mut().push(content);
    bubble.append(&view);
}

pub(super) fn ready(state: &AppState, key: String, path: String) {
    if state.emoji_textures.borrow().contains_key(&key) {
        return;
    }
    let Ok(pixbuf) = Pixbuf::from_file_at_scale(path, 28, 28, true) else {
        return;
    };
    state
        .emoji_textures
        .borrow_mut()
        .insert(key.clone(), gdk::Texture::for_pixbuf(&pixbuf));
    for content in state.emoji_views.borrow().iter() {
        if content.emojis.iter().any(|emoji| emoji.key == key) {
            render(&state.emoji_textures.borrow(), content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emoji_ranges_keep_unicode_and_reject_overlapping_or_invalid_offsets() {
        let text = "😀前(顔)後";
        let emojis = vec![
            LineEmoji {
                start: 2,
                end: 5,
                key: "face".into(),
            },
            LineEmoji {
                start: 3,
                end: 6,
                key: "overlap".into(),
            },
            LineEmoji {
                start: 6,
                end: 999,
                key: "outside".into(),
            },
        ];
        let ranges = ranges(text, &emojis);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&text[..ranges[0].0], "😀前");
        assert_eq!(&text[ranges[0].0..ranges[0].1], "(顔)");
        assert_eq!(&text[ranges[0].1..], "後");
    }
}
