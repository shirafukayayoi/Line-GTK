Unofficial native LINE client for Linux. GTK4 / Libadwaita UI with a Deno protocol sidecar ([linejs](https://github.com/evex-dev/linejs)).


![main](./assets/761546283_1013837091419784_4898538377611644175_n.jpg)

---

## Implementation

- [x] QR login (additional device)
- [x] Chats, messages, stickers, friends
- [x] メッセージ横の矢印ボタン・右クリックから引用付きのテキスト返信（リプライ）
- [x] Photos, video, voice messages, files
- [x] Paste and drag-and-drop attachments
- [x] In-app viewers (image, video, PDF, text)
- [x] Downloads, tray, Thai / English, themes
- [x] Discord RPC
- [ ] Voice call (WIP)
- [ ] Account switcher

メッセージ横の矢印ボタンを押すと引用元が選ばれ、そのまま入力欄で返信を書けます。右クリックの「リプライ」からも選べます。×ボタンまたはEscで取り消せます。別のトークへ移ると引用元を解除します。

---

## Requirements

| Dependency | Notes |
| --- | --- |
| Rust | stable toolchain |
| GTK4 + Libadwaita | `pacman -S gtk4 libadwaita` |
| Deno | Build-time protocol compiler; source-tree runs use `deno` or `DENO` |
| ffmpeg / ffprobe | voice record, video thumbs |
| Optional | `pdftoppm`, `pdftotext` for PDF preview |

Wayland and X11 both work. Tested mainly on Arch-based desktops.

---

## Install (Arch / AUR)

```bash
# Development snapshot
yay -S line-gtk-git

# Tagged release
yay -S line-gtk
```

## Install (Flatpak bundle)
```bash
flatpak install --user ./line-gtk-0.1.2-x86_64.flatpak
flatpak run dev.linegtk.LineGtk
```

## Build and run

```bash
git clone https://github.com/MidnightTale/Line-GTK.git
cd Line-GTK
cargo build --release
./target/release/line-gtk
```

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
deno task --config protocol/deno.json check
```

---

## Warning

This is an unofficial client. Using it may violate LINE Terms of Service. Prefer a secondary account or device session. Expect breakage when LINE changes login or crypto.

---

## 下書き・未送信本文・トーク内検索

入力中の本文と引用先はアカウント・トーク別にこのPCへ保存され、トーク切り替えや次回起動時に復元されます。保存できない場合は本文を残し、送信・トーク切り替えを止めてコピーを案内します。

通常のテキスト送信に失敗した場合は本文を保持し、入力欄横の未送信一覧からコピーや編集への復元ができます。送信中のまま終了したものは「結果不明」として残します。結果不明の本文を自動再送することはありません。復元先に別の下書きがある場合は上書きしません。写真・スタンプなどの送信失敗は対象外です。

Ctrl＋Fまたはトーク上部の検索ボタンで、現在のトークで読み込み済みの本文を検索できます。Enterで次、Shift＋Enterで前の一致へ移動し、Escで閉じます。検索からサーバーの過去の履歴を追加取得することはありません。

保存先はデータフォルダーの `writing-state.json` です。通常の送信操作はEnterのままです。

## Acknowledgments

Protocol and E2EE talk to LINE through [linejs](https://github.com/evex-dev/linejs) by [evex-dev](https://github.com/evex-dev). This app is a GTK frontend around that library; it is not affiliated with LINE Corporation or evex-dev.

---

## License

[GPL-3.0-or-later](LICENSE)
