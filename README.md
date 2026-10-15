Unofficial native LINE client for Linux. GTK4 / Libadwaita UI with a Deno protocol sidecar ([linejs](https://github.com/evex-dev/linejs)).

> このforkの開発ブランチは `main` です。日本語UI・フォント、メッセージ復号・グループ名、メールログインの変更を統合して開発しています。
> トーク一覧のプレビューは本文の取得時にも更新し、背景取得は同時2件で一覧全体を処理します。最新の本文を取得できない場合は「タップして開く」に戻し、読み込み中の表示を残しません。

![main](./assets/761546283_1013837091419784_4898538377611644175_n.jpg)

---

## Implementation

- [x] QR login (additional device)
- [x] メールアドレス・パスワードでログイン（スマホでの認証番号入力に対応）
- [x] Chats, messages, stickers, friends
- [x] トークを「全部・友だち・グループ・公式アカウント」で絞り込み、カテゴリ内を検索
- [x] Photos, video, voice messages, files
- [x] Paste and drag-and-drop attachments
- [x] In-app viewers (image, video, PDF, text)
- [x] Downloads, tray, Thai / English / Japanese, themes
- [x] Discord RPC
- [ ] Voice call (WIP)
- [ ] Account switcher

---

## Requirements

メールログインはPCの追加デバイスとして認証します。スマホ側で認証番号の入力が必要になる場合があります。
パスワードは保存せず、次回はPC用の認証情報でセッションを復元します。
音声通話の試験機能を使う場合はQRコードでログインしてください。

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

設定の表示言語で「日本語」を選択できます。標準フォントを使用している場合は
「Noto Sans CJK JP」に切り替え、指定済みの独自フォントは維持します。
Arch Linuxでは `noto-fonts-cjk` をインストールしてください。

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

## Acknowledgments

Protocol and E2EE talk to LINE through [linejs](https://github.com/evex-dev/linejs) by [evex-dev](https://github.com/evex-dev). This app is a GTK frontend around that library; it is not affiliated with LINE Corporation or evex-dev.

---

## License

[GPL-3.0-or-later](LICENSE)

## メッセージの復号とグループ情報

本文が `(none)` になる問題に対応するため、linejs 3.5.0を使用します。
以前のログインで誤った鍵が保存されていた場合は、設定の「アカウント」または
復号失敗の吹き出しにある暗号鍵の再取得ボタンから、スマホでQR再認証してください。
保存済みの秘密鍵は保持し、正しい世代の鍵をスマホから受け取ります。
グループ情報は小分けに取得し、名前と画像を次回起動時にも表示できるよう保存します。
