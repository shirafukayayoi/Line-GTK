import { assertEquals } from "@std/assert";
import {
  callHistoryOf,
  formatCallDuration,
  isLegacyCallPreview,
  isOutdatedGroupCallPreview,
} from "../src/call_history.ts";
import { createMessageView } from "../src/message_view.ts";
import { needsChatPreview } from "../src/chat_previews.ts";

function messageView() {
  return createMessageView({
    normType: (value) => String(value ?? "NONE"),
    existingFullImage: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    mediaDest: () => "",
    thumbDest: () => "",
    isValidAudioFile: () => Promise.resolve(false),
    uiMediaPath: () => Promise.resolve(null),
    stickerDir: "",
  });
}

Deno.test("旧版の通話履歴から時間を取り出し、結果コードの意味は推測しない", () => {
  assertEquals(
    callHistoryOf({
      contentType: "CALL",
      text: "Call History : 136000 millisecs, Result: 16",
    }),
    { video: false, status: "completed", durationMs: 136000 },
  );
  assertEquals(
    callHistoryOf({
      contentType: "CALL",
      text: "Call History : 0 millisecs, Result: 77",
    }),
    { video: false, status: "unknown", durationMs: 0 },
  );
  assertEquals(formatCallDuration(136000), "2:16");
  assertEquals(formatCallDuration(3723000), "1:02:03");
});

Deno.test("明示された未応答とビデオ種別を保持し、通常の本文は変換しない", () => {
  assertEquals(
    callHistoryOf({
      contentType: "NONE",
      contentMetadata: {
        ORGCONTP: "CALL",
        RESULT: "CANCELED",
        TYPE: "V",
        DURATION: "0",
      },
    }),
    { video: true, status: "unanswered", durationMs: 0 },
  );
  assertEquals(
    callHistoryOf({
      contentType: "NONE",
      text: "Call History : 136000 millisecs, Result: 16",
    }),
    null,
  );
  assertEquals(
    callHistoryOf({
      contentType: 6,
      contentMetadata: { DURATION: "-1", RESULT: "未確認" },
    }),
    { video: false, status: "unknown", durationMs: null },
  );
});

Deno.test("履歴カードと一覧用に、既存のメッセージキャッシュも構造化する", async () => {
  const view = messageView();
  const [message] = await view.forUiMessages([{
    id: "fixture",
    contentType: "CALL",
    mine: false,
    text: "Call History : 136000 millisecs, Result: 16",
    needsMedia: true,
  }]);
  assertEquals(message.callHistory, {
    video: false,
    status: "completed",
    durationMs: 136000,
  });
  assertEquals(message.needsMedia, false);
});

Deno.test("日本語の一覧で着信・発信・不在着信と通話時間が分かる", () => {
  const previous = Deno.env.get("LINE_GTK_LANG");
  const view = messageView();
  try {
    Deno.env.set("LINE_GTK_LANG", "ja");
    const incoming = { contentType: "CALL", mine: false, durationMs: 136000 };
    assertEquals(view.previewLine(incoming), "相手: 着信がありました · 2:16");
    assertEquals(
      view.previewLine({ ...incoming, mine: true }),
      "自分: 発信しました · 2:16",
    );
    assertEquals(
      view.previewBody({
        contentType: "CALL",
        callHistory: { video: false, status: "unanswered", durationMs: 0 },
      }),
      "不在着信",
    );
    assertEquals(
      view.previewBody({
        contentType: "CALL",
        callHistory: { video: true, status: "unanswered", durationMs: 0 },
      }),
      "ビデオ通話の不在着信",
    );
    assertEquals(
      view.previewBody({ contentType: "CALL", durationMs: 0 }),
      "着信がありました",
    );
    Deno.env.set("LINE_GTK_LANG", "en");
    assertEquals(view.previewBody(incoming), "Incoming call · 2:16");
    Deno.env.set("LINE_GTK_LANG", "th");
    assertEquals(view.previewBody(incoming), "มีสายเรียกเข้า · 2:16");
  } finally {
    if (previous === undefined) Deno.env.delete("LINE_GTK_LANG");
    else Deno.env.set("LINE_GTK_LANG", previous);
  }
});

Deno.test("一覧に保存された旧内部表現は再取得して読みやすい表示へ移行する", () => {
  const preview = "They: Call History : 136000 millisecs, Result: 16";
  assertEquals(isLegacyCallPreview(preview), true);
  assertEquals(
    needsChatPreview({ mid: "u-fixture", preview, lastActivity: 20 }),
    true,
  );
  assertEquals(isLegacyCallPreview("Call Historyについて相談"), false);
});

Deno.test("グループ・複数人トークは送受信方向や未応答にかかわらずグループ通話と表示する", async () => {
  const previous = Deno.env.get("LINE_GTK_LANG");
  const view = messageView();
  try {
    Deno.env.set("LINE_GTK_LANG", "ja");
    for (const to of ["c-group", "r-room"]) {
      for (const mine of [true, false]) {
        for (const status of ["completed", "unanswered", "unknown"]) {
          const message = {
            to,
            mine,
            contentType: "CALL",
            callHistory: { status, durationMs: 0 },
          };
          assertEquals(view.previewLine(message), "グループ通話");
          const [ui] = await view.forUiMessages([message]);
          assertEquals((ui.callHistory as { group: boolean }).group, true);
        }
      }
    }
    assertEquals(
      view.previewLine({
        to: "c-group",
        contentType: "CALL",
        durationMs: 10147000,
      }),
      "グループ通話 · 2:49:07",
    );
    const video = {
      contentType: "CALL",
      callHistory: {
        group: true,
        video: true,
        status: "unanswered",
        durationMs: 0,
      },
    };
    assertEquals(view.previewLine(video), "グループビデオ通話");
    Deno.env.set("LINE_GTK_LANG", "en");
    assertEquals(view.previewLine(video), "Group video call");
    Deno.env.set("LINE_GTK_LANG", "th");
    assertEquals(view.previewLine(video), "วิดีโอคอลกลุ่ม");
  } finally {
    if (previous === undefined) Deno.env.delete("LINE_GTK_LANG");
    else Deno.env.set("LINE_GTK_LANG", previous);
  }
});

Deno.test("保存済みの個別通話風プレビューはグループだけ再取得し、通常本文を変換しない", () => {
  for (
    const title of [
      "着信がありました",
      "発信しました",
      "不在着信",
      "ビデオ通話の不在着信",
      "Incoming call",
      "Unanswered video call",
      "มีสายเรียกเข้า",
      "สายวิดีโอที่ไม่ได้รับ",
    ]
  ) {
    const preview = `相手: ${title} · 2:49:07`;
    assertEquals(isOutdatedGroupCallPreview("c-group", preview), true);
    assertEquals(
      needsChatPreview({ mid: "c-group", preview, lastActivity: 20 }),
      true,
    );
    assertEquals(
      needsChatPreview({ mid: "u-peer", preview, lastActivity: 20 }),
      false,
    );
  }
  assertEquals(
    isOutdatedGroupCallPreview("c-group", "グループ通話 · 2:49:07"),
    false,
  );
  assertEquals(
    callHistoryOf({
      contentType: "NONE",
      to: "c-group",
      text: "着信がありました",
    }),
    null,
  );
});
