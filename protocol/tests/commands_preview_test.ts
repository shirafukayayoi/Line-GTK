import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { policyFor } from "../src/cache_policy.ts";
import { createChatPreviews } from "../src/chat_previews.ts";

type Json = Record<string, unknown>;

function commandFixture(source: "通信" | "メモリ" | "ディスク") {
  const message = { id: "2", createdTime: 20, text: "読み込んだ最新の本文" };
  const chat = {
    mid: "u-fixture",
    name: "検証用トーク",
    kind: "dm",
    avatarPath: null,
    lastActivity: 20,
    unread: 0,
    preview: "",
  };
  const chatCache = { at: Date.now(), chats: [chat] };
  const events: Json[] = [];
  const okResults: unknown[] = [];
  const previews = createChatPreviews({
    getChats: () => chatCache.chats,
    previewLine: (message) => `相手: ${message.text}`,
    fallback: () => "タップして開く",
    emit: (payload) => events.push({ event: "chat_preview", ...payload }),
    save: () => Promise.resolve(),
  });
  let networkCalls = 0;
  const client = {
    base: {
      talk: {
        getPreviousMessagesV2WithRequest: () => {
          networkCalls++;
          return Promise.resolve([message]);
        },
      },
    },
  } as unknown as Client;
  const noOp = () => Promise.resolve();
  const runtime: CommandRuntime = {
    getClient: () => client,
    setClient: () => {},
    isListening: () => false,
    setAuthDead: () => {},
    getChatCache: () => chatCache,
    setChatCache: () => {},
    msgCache: new Map(
      source === "メモリ"
        ? [[chat.mid, { at: Date.now(), messages: [message] }]]
        : [],
    ),
    contactIndex: new Map(),
    boxCursor: new Map([[chat.mid, { messageId: 2, deliveredTime: 20 }]]),
    avatarDir: "",
    chatCachePath: "",
    storagePath: "",
    lineDevice: "DESKTOPWIN",
    lineVersion: "",
    refreshCachePolicy: () => Promise.resolve(policyFor("smart")),
    cachePolicy: () => policyFor("smart"),
    dedupe: (_key, operation) => operation(),
    storeBoxCursor: () => {},
    existingFile: () => Promise.resolve(null),
    loadDiskContacts: noOp,
    saveDiskContacts: noOp,
    refreshContactIndex: noOp,
    hydrateAvatars: noOp,
    hydratePreviews: noOp,
    updateChatPreview: previews.updateFromMessages,
    bumpMediaEpoch: () => 0,
    loadDiskMessages: () =>
      Promise.resolve(source === "ディスク" ? [message] : null),
    saveDiskMessages: noOp,
    forUiMessages: (messages) => Promise.resolve(messages),
    hydrateMedia: noOp,
    summarizeTalkMessage: () => Promise.resolve(message),
    summarizeRawMessage: (raw) => Promise.resolve(raw as Json),
    saveAuth: noOp,
    loadAuth: () => Promise.resolve(null),
    loadAuthDevice: () => Promise.resolve("DESKTOPWIN"),
    clearAuth: noOp,
    myProfilePayload: () => Promise.resolve({}),
    patchE2eeGuards: () => {},
    startListen: () => {},
    sentMessagePayload: () => ({}),
    touchChatPreviewFromMessage: () => {},
    emitEvent: (event, payload) => events.push({ event, ...payload }),
    ok: (_id, result) => okResults.push(result),
    fail: (_id, error) => {
      throw new Error(error);
    },
  };
  return {
    commands: createCommandService(runtime),
    chat,
    events,
    okResults,
    networkCalls: () => networkCalls,
  };
}

for (const source of ["通信", "メモリ", "ディスク"] as const) {
  Deno.test(`${source}から本文を開くと一覧の読み込み中も解消する`, async () => {
    const fixture = commandFixture(source);
    await fixture.commands.fetchMessages(1, fixture.chat.mid);
    assertEquals(fixture.chat.preview, "相手: 読み込んだ最新の本文");
    assertEquals(
      fixture.events.filter((event) => event.event === "chat_preview"),
      [{
        event: "chat_preview",
        mid: fixture.chat.mid,
        preview: "相手: 読み込んだ最新の本文",
        lastActivity: 20,
      }],
    );
    assertEquals(fixture.okResults.length, 1);
    assertEquals(fixture.networkCalls(), source === "メモリ" ? 0 : 1);
  });
}
