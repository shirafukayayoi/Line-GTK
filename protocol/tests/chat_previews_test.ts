import { assertEquals } from "@std/assert";
import { createChatPreviews, type PreviewChat } from "../src/chat_previews.ts";

type Json = Record<string, unknown>;

export function previewFixture(chats: PreviewChat[]) {
  const events: PreviewChat[] = [];
  let saved = 0;
  const previews = createChatPreviews({
    getChats: () => chats,
    previewLine: (message) => `相手: ${message.text}`,
    fallback: () => "タップして開く",
    emit: (chat) => events.push({ ...chat }),
    save: () => {
      saved++;
      return Promise.resolve();
    },
  });
  return { previews, events, saved: () => saved };
}

Deno.test("本文を読み込むと一覧プレビューも更新し、再起動用に保存する", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews, events, saved } = previewFixture([chat]);
  const messages: Json[] = [
    { id: "2", createdTime: 20, text: "最新の本文" },
    { id: "1", createdTime: 10, text: "古い本文" },
  ];
  await previews.updateFromMessages(chat.mid, messages);
  assertEquals(chat.preview, "相手: 最新の本文");
  assertEquals(events, [{ ...chat }]);
  assertEquals(saved(), 1);
});

Deno.test("同時刻の本文は安全な整数範囲を超えるIDでも新しい順に選ぶ", async () => {
  const chat = { mid: "c-fixture", preview: "", lastActivity: 20 };
  const { previews } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, [
    { id: "9007199254740993", createdTime: 20, text: "新しい本文" },
    { id: "9007199254740992", createdTime: 20, text: "古い本文" },
  ]);
  assertEquals(chat.preview, "相手: 新しい本文");
});

Deno.test("古い履歴と空の応答で新着プレビューを上書きしない", async () => {
  const chat = { mid: "u-fixture", preview: "相手: 新着", lastActivity: 30 };
  const { previews, events, saved } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, [
    { createdTime: 20, text: "古い履歴" },
  ]);
  await previews.updateFromMessages(chat.mid, []);
  assertEquals(chat.preview, "相手: 新着");
  assertEquals(events, []);
  assertEquals(saved(), 0);
});

Deno.test("本文が空の場合は読み込み中を終了する", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, []);
  assertEquals(chat.preview, "タップして開く");
});
