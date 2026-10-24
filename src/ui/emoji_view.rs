use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EmojiView;

    #[glib::object_subclass]
    impl ObjectSubclass for EmojiView {
        const NAME: &'static str = "LineGtkEmojiView";
        type Type = super::EmojiView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for EmojiView {}
    impl TextViewImpl for EmojiView {}

    impl WidgetImpl for EmojiView {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let view = self.obj();
            let width = (text_size(view.upcast_ref(), None).0
                + view.left_margin()
                + view.right_margin()
                + 1)
            .clamp(28, 340);
            if orientation == gtk::Orientation::Horizontal {
                return (28, width, -1, -1);
            }

            // TextView 標準の高さは、未配置時の幅1pxで計算されたキャッシュを
            // 含み得る。仮想一覧の位置補正を避けるため、要求された幅で測る。
            let available = if for_size < 0 { width } else { for_size };
            let height = text_size(view.upcast_ref(), Some(available)).1
                + view.top_margin()
                + view.bottom_margin();
            (height, height, -1, -1)
        }
    }
}

glib::wrapper! {
    pub struct EmojiView(ObjectSubclass<imp::EmojiView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::AccessibleText, gtk::Scrollable;
}

// TextView と同じく段落単位で測定・丸める。全段落を一括測定すると、
// 絵文字の行高と端数の違いが蓄積して長文の末尾が欠けることがある。
fn text_size(view: &gtk::TextView, width: Option<i32>) -> (i32, i32) {
    let buffer = view.buffer();
    let mut size = (0, 0);
    for line in 0..buffer.line_count() {
        let mut iter = buffer.iter_at_line(line).expect("バッファ内の段落");
        let mut end = iter;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let text = buffer.slice(&iter, &end, true);
        let layout = view.create_pango_layout(Some(&text));
        let attributes = pango::AttrList::new();
        for (index, ch) in text.char_indices() {
            if let Some(paintable) = iter.paintable() {
                let width = paintable.intrinsic_width().max(1) * pango::SCALE;
                let height = paintable.intrinsic_height().max(1) * pango::SCALE;
                let rect = pango::Rectangle::new(0, -height, width, height);
                let mut shape = pango::AttrShape::new(&rect, &rect);
                shape.set_start_index(index as u32);
                shape.set_end_index((index + ch.len_utf8()) as u32);
                attributes.insert(shape);
            }
            iter.forward_char();
        }
        layout.set_attributes(Some(&attributes));
        layout.set_wrap(pango::WrapMode::WordChar);
        if let Some(width) = width {
            // TextView 自身もカーソル用に1pxを確保して折り返す。
            layout.set_width(
                (width - view.left_margin() - view.right_margin() - 1).max(1) * pango::SCALE,
            );
        }
        size.0 = size.0.max(layout.pixel_size().0);
        size.1 += (layout.size().1 + pango::SCALE / 2) / pango::SCALE;
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "GTK用の専用画面で実行する"]
    fn height_matches_wrapped_text_and_inline_images_before_allocation() {
        gtk::test_synced(|| {
            let view: EmojiView = glib::Object::builder()
                .property("wrap-mode", gtk::WrapMode::WordChar)
                .build();
            let pixbuf =
                gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 28, 28).unwrap();
            pixbuf.fill(0x22bb44ff);
            let texture = gtk::gdk::Texture::for_pixbuf(&pixbuf);
            for text in [
                "短い本文",
                "日本語と😀を含む長文。\n".repeat(30).as_str(),
                "改行なしの長い本文😀".repeat(60).as_str(),
                "空行の前\n\n\n空行の後\r\nCRLFの後\u{2029}段落区切りの後",
                "",
            ] {
                let buffer = view.buffer();
                buffer.set_text(text);
                let mut iter = buffer.start_iter();
                buffer.insert_paintable(&mut iter, &texture);
                buffer.insert_paintable(&mut iter, &texture);
                view.queue_resize();
                let (_, width, _, _) = view.measure(gtk::Orientation::Horizontal, -1);
                assert!((28..=340).contains(&width));
                let (_, height, _, _) = view.measure(gtk::Orientation::Vertical, width);
                assert!(height < 2000, "未配置時に高さが膨張しています: {height}");
                view.allocate(width, height, -1, None);
                let end = view.iter_location(&buffer.end_iter());
                assert!(
                    end.y() + end.height() <= height,
                    "末尾が切れています: 実際の末尾={}、測定高さ={height}",
                    end.y() + end.height()
                );
                let (_, narrow, _, _) = view.measure(gtk::Orientation::Vertical, 80);
                if width >= 80 {
                    assert!(
                        narrow >= height,
                        "幅={width}、高さ={height}、幅80時={narrow}"
                    );
                }
                view.allocate(80, narrow, -1, None);
                let end = view.iter_location(&buffer.end_iter());
                assert!(
                    end.y() + end.height() <= narrow,
                    "狭い幅で末尾が切れています: 末尾={}、測定高さ={narrow}、割当幅={}、本文={text:?}",
                    end.y() + end.height(),
                    view.width()
                );
            }
        });
    }
}
