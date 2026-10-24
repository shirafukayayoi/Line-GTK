import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { policyFor } from "../src/cache_policy.ts";
import { createChatPreviews } from "../src/chat_previews.ts";
import { atomicWriteJson } from "../src/storage.ts";
import { readReceiptPermission } from "../src/read_policy.ts";

type Json = Record<string, unknown>;

function commandFixture(
  source: "通信" | "メモリ" | "ディスク",
  chatCachePath = "",
) {
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
  let chatCache: ReturnType<CommandRuntime["getChatCache"]> = {
    at: Date.now(),
    chats: [chat],
  };
  const events: Json[] = [];
  const okResults: unknown[] = [];
  const previews = createChatPreviews({
    getChats: () => chatCache?.chats ?? [],
    previewLine: (message) => `相手: ${message.text}`,
    fallback: () => "タップして開く",
    emit: (payload) => events.push({ event: "chat_preview", ...payload }),
    save: () => Promise.resolve(),
  });
  let networkCalls = 0;
  let marks = 0;
  let sequenceHook = () => Promise.resolve(1);
  let hydrated = 0;
  let receiptsRefreshed = 0;
  let activity = 20;
  const client = {
    base: {
      getReqseq: () => sequenceHook(),
      talk: {
        sendChatChecked: () => {
          marks++;
          return Promise.resolve();
        },
        getMessageBoxes: () =>
          Promise.resolve({
            messageBoxes: [{
              id: chat.mid,
              lastDeliveredMessageId: { messageId: 2, deliveredTime: activity },
            }],
          }),
        getAllChatMids: () => Promise.resolve({ memberChatMids: [] }),
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
    setChatCache: (next) => {
      chatCache = next;
    },
    msgCache: new Map(
      source === "メモリ"
        ? [[chat.mid, { at: Date.now(), messages: [message] }]]
        : [],
    ),
    contactIndex: new Map(),
    boxCursor: new Map([[chat.mid, { messageId: 2, deliveredTime: 20 }]]),
    avatarDir: "",
    chatCachePath,
    saveChatCache: () => atomicWriteJson(chatCachePath, chatCache),
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
    hydratePreviews: () => {
      hydrated++;
      return Promise.resolve();
    },
    updateChatPreview: previews.updateFromMessages,
    readReceiptPermission: () => Promise.resolve("allowed"),
    refreshReadReceipts: () => {
      receiptsRefreshed++;
      return Promise.resolve();
    },
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
    hydrated: () => hydrated,
    receiptsRefreshed: () => receiptsRefreshed,
    marks: () => marks,
    setSequenceHook: (hook: () => Promise<number>) => {
      sequenceHook = hook;
    },
    setActivity: (next: number) => {
      activity = next;
    },
    runtime,
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
    assertEquals(fixture.receiptsRefreshed(), 1);
    assertEquals(fixture.networkCalls(), source === "メモリ" ? 0 : 1);
  });
}

Deno.test("復帰時の強制取得ではメモリと古い最新位置を使わず未取得の本文を取得する", async () => {
  const fixture = commandFixture("メモリ");
  const calls: unknown[] = [];
  fixture.runtime.storeBoxCursor = (mid, value) => {
    fixture.runtime.boxCursor.set(mid, value as never);
  };
  const client = fixture.runtime.getClient()!;
  client.base.talk.getMessageBoxes = () =>
    Promise.resolve({
      messageBoxes: [{
        id: fixture.chat.mid,
        lastDeliveredMessageId: { messageId: 99, deliveredTime: 100 },
      }],
    } as never);
  client.base.talk.getPreviousMessagesV2WithRequest = (params) => {
    calls.push(params);
    return Promise.resolve(
      [{ id: "99", createdTime: 100, text: "スリープ中の新着" }] as never,
    );
  };
  fixture.runtime.summarizeTalkMessage = (message) =>
    Promise.resolve(message as unknown as Json);
  fixture.runtime.summarizeRawMessage = (raw) => Promise.resolve(raw as Json);
  await createCommandService(fixture.runtime).fetchMessages(
    1,
    fixture.chat.mid,
    80,
    true,
    true,
  );
  assertEquals(
    (calls[0] as { request: { endMessageId: unknown } }).request.endMessageId,
    {
      messageId: 99,
      deliveredTime: 100,
    },
  );
  assertEquals(
    (fixture.okResults[0] as { messages: Json[] }).messages[0].id,
    "99",
  );
  assertEquals(
    fixture.events.filter((event) =>
      event.event === "progress" || event.event === "messages"
    ),
    [],
  );
  assertEquals(fixture.marks(), 0);
});

Deno.test("復帰時に通信できない場合は既存の一覧を空の成功結果で上書きしない", async () => {
  const fixture = commandFixture("メモリ");
  const failures: string[] = [];
  fixture.runtime.fail = (_id, error) => failures.push(error);
  fixture.runtime.getClient()!.base.talk.getMessageBoxes = () =>
    Promise.reject(new Error("復帰直後はオフライン"));
  const before = fixture.runtime.getChatCache();
  await createCommandService(fixture.runtime).listChats(1, true, true);
  assertEquals(failures, ["chat_sync_failed"]);
  assertEquals(fixture.okResults, []);
  assertEquals(fixture.runtime.getChatCache(), before);
  assertEquals(fixture.events, []);
});

Deno.test("一覧がメモリキャッシュでも未取得のプレビューを補完する", async () => {
  const fixture = commandFixture("メモリ");
  await fixture.commands.listChats(1);
  assertEquals(fixture.hydrated(), 1);
  assertEquals(fixture.runtime.getChatCache()?.chats[0].preview, "");
});

Deno.test("再起動後の一覧更新で保存済みのプレビューを失わない", async () => {
  const directory = await Deno.makeTempDir();
  const fixture = commandFixture("通信", `${directory}/chats.json`);
  try {
    await Deno.writeTextFile(
      fixture.runtime.chatCachePath,
      JSON.stringify({
        at: Date.now(),
        chats: [{ ...fixture.chat, preview: "相手: 保存済みの本文" }],
      }),
    );
    fixture.runtime.setChatCache(null);
    await fixture.commands.listChats(1);
    assertEquals(
      fixture.runtime.getChatCache()?.chats[0].preview,
      "相手: 保存済みの本文",
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("新着のある一覧では古いプレビューを再取得の対象に戻す", async () => {
  const directory = await Deno.makeTempDir();
  const fixture = commandFixture("通信", `${directory}/chats.json`);
  fixture.chat.preview = "相手: 古い本文";
  fixture.setActivity(30);
  try {
    await fixture.commands.listChats(1, true);
    assertEquals(fixture.runtime.getChatCache()?.chats[0].preview, "");
    assertEquals(fixture.runtime.getChatCache()?.chats[0].lastActivity, 30);
    assertEquals(fixture.hydrated(), 1);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("既読停止中はRPCと未読件数の更新を行わない", async () => {
  const fixture = commandFixture("通信");
  fixture.chat.unread = 3;
  fixture.runtime.readReceiptPermission = () => Promise.resolve("paused");
  await fixture.commands.markRead(1, fixture.chat.mid, "9007199254740993");
  assertEquals(fixture.marks(), 0);
  assertEquals(fixture.chat.unread, 3);
  assertEquals(fixture.okResults, [{ marked: false, skipped: "paused" }]);
});

Deno.test("順序番号の取得中に停止をオンにしても既読を送らない", async () => {
  const fixture = commandFixture("通信");
  let paused = false;
  fixture.runtime.readReceiptPermission = () =>
    Promise.resolve(paused ? "paused" : "allowed");
  fixture.setSequenceHook(() => {
    paused = true;
    return Promise.resolve(1);
  });
  await fixture.commands.markRead(1, fixture.chat.mid, "9007199254740993");
  assertEquals(fixture.marks(), 0);
  assertEquals(fixture.okResults, [{ marked: false, skipped: "paused" }]);
});

Deno.test("停止と自動既読オフの解除後は、新しい要求だけ既読を送る", async () => {
  const fixture = commandFixture("通信");
  fixture.chat.unread = 3;
  fixture.runtime.readReceiptPermission = () =>
    Promise.resolve("auto_disabled");
  await fixture.commands.markRead(1, fixture.chat.mid, "50");
  assertEquals(fixture.marks(), 0);
  fixture.runtime.readReceiptPermission = () => Promise.resolve("allowed");
  assertEquals(fixture.marks(), 0);
  await fixture.commands.markRead(2, fixture.chat.mid, "50");
  assertEquals(fixture.marks(), 1);
  assertEquals(fixture.chat.unread, 0);
});

Deno.test("友だち別の保存済み停止をRPC直前にも確認し、他の相手は通常どおり送る", async () => {
  const directory = await Deno.makeTempDir();
  const path = `${directory}/settings.json`;
  const fixture = commandFixture("通信");
  const settings = {
    read_receipts_paused_chats: {} as Record<string, string[]>,
  };
  const save = () => Deno.writeTextFile(path, JSON.stringify(settings));
  fixture.chat.unread = 3;
  fixture.runtime.readReceiptPermission = (mid) =>
    readReceiptPermission(path, "u-self", mid);
  try {
    settings.read_receipts_paused_chats["u-self"] = [fixture.chat.mid];
    await save();
    await fixture.commands.markRead(1, fixture.chat.mid, "50");
    assertEquals(fixture.marks(), 0);
    assertEquals(fixture.chat.unread, 3);
    assertEquals(fixture.okResults.at(-1), {
      marked: false,
      skipped: "chat_paused",
    });
    await fixture.commands.markRead(2, "u-other", "50");
    assertEquals(fixture.marks(), 1);
    settings.read_receipts_paused_chats = {};
    await save();
    fixture.setSequenceHook(async () => {
      settings.read_receipts_paused_chats["u-self"] = [fixture.chat.mid];
      await save();
      return 1;
    });
    await fixture.commands.markRead(3, fixture.chat.mid, "51");
    assertEquals(fixture.marks(), 1);
    assertEquals(fixture.chat.unread, 3);
    assertEquals(fixture.okResults.at(-1), {
      marked: false,
      skipped: "chat_paused",
    });
    settings.read_receipts_paused_chats = {};
    await save();
    assertEquals(fixture.marks(), 1);
    fixture.setSequenceHook(() => Promise.resolve(1));
    await fixture.commands.markRead(4, fixture.chat.mid, "52");
    assertEquals(fixture.marks(), 2);
    assertEquals(fixture.chat.unread, 0);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("一回だけの既読は指定した位置だけを送り、その後の自動既読は停止したままにする", async () => {
  const fixture = commandFixture("通信");
  const recorded: string[][] = [];
  fixture.chat.unread = 3;
  fixture.runtime.readReceiptPermission = () => Promise.resolve("paused");
  fixture.runtime.recordOwnRead = (mid, through) =>
    recorded.push([mid, through]);
  await fixture.commands.markReadOnce(1, fixture.chat.mid, "9007199254740993");
  assertEquals(fixture.marks(), 1);
  assertEquals(recorded, [[fixture.chat.mid, "9007199254740993"]]);
  // 未読件数は未読同期で更新する。新着を考慮せず0に書き換えない。
  assertEquals(fixture.chat.unread, 3);
  await fixture.commands.markRead(2, fixture.chat.mid, "9007199254740994");
  assertEquals(fixture.marks(), 1);
  assertEquals(fixture.okResults.at(-1), { marked: false, skipped: "paused" });
});

Deno.test("未表示トークの一回既読はサーバーの最新位置を取得し、不正な位置は送らない", async () => {
  const fixture = commandFixture("通信");
  const recorded: string[] = [];
  fixture.runtime.readReceiptPermission = () => Promise.resolve("chat_paused");
  fixture.runtime.recordOwnRead = (_mid, through) => recorded.push(through);
  await fixture.commands.markReadOnce(1, fixture.chat.mid);
  assertEquals(recorded, ["2"]);
  assertEquals(fixture.marks(), 1);
  for (const through of ["event:50", "-1", "0", "bad"]) {
    await fixture.commands.markReadOnce(2, fixture.chat.mid, through);
  }
  assertEquals(fixture.marks(), 1);
});

Deno.test("一回既読でも、順序番号の待機中のセッション変更と通信失敗では未読を消さない", async () => {
  const fixture = commandFixture("通信");
  let recorded = 0;
  fixture.chat.unread = 3;
  fixture.runtime.recordOwnRead = () => {
    recorded++;
  };
  const original = fixture.runtime.getClient();
  fixture.setSequenceHook(() => {
    fixture.runtime.getClient = () => null;
    return Promise.resolve(1);
  });
  await fixture.commands.markReadOnce(1, fixture.chat.mid, "50");
  assertEquals(fixture.marks(), 0);
  assertEquals(recorded, 0);
  assertEquals(fixture.chat.unread, 3);
  fixture.runtime.getClient = () => original;
  fixture.setSequenceHook(() => Promise.reject(new Error("模擬通信失敗")));
  await fixture.commands.markReadOnce(2, fixture.chat.mid, "50");
  assertEquals(fixture.marks(), 0);
  assertEquals(recorded, 0);
  assertEquals(fixture.chat.unread, 3);
  assertEquals((fixture.okResults.at(-1) as { marked: boolean }).marked, false);
});

for (const route of ["plain", "encrypted", "chat"] as const) {
  Deno.test(`返信先IDを${route}の送信経路でも維持する`, async () => {
    const fixture = commandFixture("メモリ");
    const calls: Record<string, unknown>[] = [];
    const client = fixture.runtime.getClient()!;
    client.base.talk.sendMessage = (options) => {
      calls.push(options as Record<string, unknown>);
      if (route === "chat" || (route === "encrypted" && !options.e2ee)) {
        return Promise.reject(new Error("経路の検証"));
      }
      return Promise.resolve({ id: "9007199254740994" } as never);
    };
    client.getChat = (() =>
      Promise.resolve({
        sendMessage: (options: Record<string, unknown>) => {
          calls.push(options);
          return Promise.resolve({ raw: { id: "9007199254740994" } });
        },
      })) as unknown as typeof client.getChat;
    const results: unknown[] = [];
    const commands = createCommandService({
      ...fixture.runtime,
      summarizeTalkMessage: () =>
        Promise.resolve({ id: "9007199254740994", text: "返信" }),
      sentMessagePayload: () => ({ id: "9007199254740994", text: "返信" }),
      ok: (_id, result) => results.push(result),
    });
    await commands.sendMessage(1, fixture.chat.mid, "返信", "9007199254740993");
    assertEquals(
      calls.length,
      route === "plain" ? 1 : route === "encrypted" ? 2 : 3,
    );
    for (const call of calls) {
      assertEquals(call.relatedMessageId, "9007199254740993");
    }
    assertEquals(
      (results[0] as { message: { replyTo: unknown } }).message.replyTo,
      { messageId: "9007199254740993" },
    );
  });
}

Deno.test("返信先が不正なら送信せず、送信失敗でも引用なしに変換しない", async () => {
  const fixture = commandFixture("メモリ");
  let sends = 0;
  const failures: string[] = [];
  const client = fixture.runtime.getClient()!;
  client.base.talk.sendMessage = () => {
    sends++;
    return Promise.reject(new Error("送信不可"));
  };
  client.getChat = (() =>
    Promise.resolve({
      sendMessage: (options: { relatedMessageId?: string }) => {
        sends++;
        assertEquals(options.relatedMessageId, "42");
        return Promise.reject(new Error("送信不可"));
      },
    })) as unknown as typeof client.getChat;
  const commands = createCommandService({
    ...fixture.runtime,
    fail: (_id, error) => failures.push(error),
  });
  await commands.sendMessage(1, fixture.chat.mid, "返信", "pending-1");
  assertEquals(sends, 0);
  assertEquals(failures, ["invalid_reply_target"]);
  await commands.sendMessage(2, fixture.chat.mid, "返信", "42");
  assertEquals(sends, 3);
  assertEquals(failures, ["invalid_reply_target", "送信不可"]);
});
for (const saveFails of [false, true]) {
  Deno.test(`本文で分かった公式分類を即時反映し、保存失敗=${saveFails}でも本文を表示する`, async () => {
    const fixture = commandFixture("通信");
    const client = fixture.runtime.getClient()!;
    client.base.talk.getPreviousMessagesV2WithRequest = () =>
      Promise.resolve([
        {
          id: "2",
          from: fixture.chat.mid,
          contentMetadata: { BOT_CHECK: "1" },
        },
      ] as never);
    let saves = 0;
    fixture.runtime.saveDiskContacts = () => {
      saves++;
      return saveFails
        ? Promise.reject(new Error("模擬保存失敗"))
        : Promise.resolve();
    };
    fixture.runtime.saveChatCache = () => Promise.resolve();
    await fixture.commands.fetchMessages(1, fixture.chat.mid);
    assertEquals(fixture.chat.kind, "bot");
    assertEquals(
      fixture.runtime.contactIndex.get(fixture.chat.mid)?.kind,
      "bot",
    );
    assertEquals(saves, 1);
    assertEquals(
      fixture.events.filter((event) => event.event === "chat_upsert").length,
      1,
    );
    assertEquals(
      (fixture.okResults[0] as { messages: Json[] }).messages.length,
      1,
    );
  });
}

Deno.test("過去ページを保存していても最新の連続した範囲から履歴を遡り始める", async () => {
  const fixture = commandFixture("メモリ");
  const messages = [1, 2, 90, 91].map((n) => ({
    id: String(n),
    createdTime: n,
    text: String(n),
    historyCursor: { messageId: String(n), deliveredTime: String(n) },
  }));
  fixture.runtime.msgCache.set(fixture.chat.mid, {
    at: Date.now(),
    messages,
    latestCount: 2,
  });
  await fixture.commands.fetchMessages(1, fixture.chat.mid, 40);
  const result = fixture.okResults[0] as {
    messages: { id: string }[];
    historyCursor: unknown;
  };
  assertEquals(result.messages.map((message) => message.id), ["90", "91"]);
  assertEquals(result.historyCursor, { messageId: "90", deliveredTime: "90" });
});
