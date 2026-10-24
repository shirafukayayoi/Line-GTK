import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import type { CommandRuntime } from "../src/commands.ts";
import {
  createHistoryService,
  cursorOf,
  historyState,
  oldestCursor,
  withHistoryCursor,
} from "../src/history.ts";

type Json = Record<string, unknown>;
const raw = (id: string, time = 10) => ({
  id,
  deliveredTime: BigInt(time),
  createdTime: time - 2,
  text: id,
});
function fixture(pages: Json[][]) {
  const calls: Json[] = [],
    results: Json[] = [],
    failures: string[] = [],
    saved: Json[][] = [],
    media: unknown[] = [];
  let client: Client | null = {
    base: {
      talk: {
        getMessageBoxes: () =>
          Promise.resolve({
            messageBoxes: [{
              id: "c-test",
              lastDeliveredMessageId: {
                messageId: 9007199254741000n,
                deliveredTime: 100n,
              },
            }],
          }),
        getPreviousMessagesV2WithRequest: ({ request }: { request: Json }) => {
          calls.push(request);
          return Promise.resolve(pages.shift() ?? []);
        },
      },
    },
  } as unknown as Client;
  const runtime = {
    getClient: () => client,
    msgCache: new Map([["c-test", {
      at: 123,
      messages: [
        withHistoryCursor(
          { id: "9007199254741000", createdTime: 100 },
          raw("9007199254741000", 100),
        ),
      ],
    }]]),
    loadDiskMessages: () => Promise.resolve(null),
    saveDiskMessages: (_mid: string, messages: Json[]) => {
      saved.push(messages);
      return Promise.resolve();
    },
    summarizeRawMessage: (m: Json) =>
      Promise.resolve({ id: m.id, createdTime: m.createdTime, text: m.text }),
    forUiMessages: (messages: Json[]) => Promise.resolve(messages),
    hydrateMedia: (_messages: Json[], _mid: string, options: unknown) => {
      media.push(options);
      return Promise.resolve();
    },
    ok: (_id: unknown, result: Json) => results.push(result),
    fail: (_id: unknown, error: string) => failures.push(error),
  } as unknown as CommandRuntime;
  return {
    runtime,
    calls,
    results,
    failures,
    saved,
    media,
    fetch: createHistoryService(runtime),
    logout: () => {
      client = null;
    },
  };
}

Deno.test("履歴の取得位置は64bit整数を丸めず配信時刻を使う", () => {
  assertEquals(cursorOf(raw("9007199254740993", 10)), {
    messageId: "9007199254740993",
    deliveredTime: "10",
  });
  assertEquals(
    cursorOf({ id: Number("9007199254740993"), deliveredTime: 10 }),
    null,
  );
  assertEquals(
    cursorOf({ id: "9223372036854775808", deliveredTime: 10 }),
    null,
  );
  assertEquals(cursorOf({ id: "1", createdTime: 10 }), null);
  assertEquals(
    oldestCursor([
      raw("5", 30),
      raw("9007199254740993", 10),
      raw("9007199254740992", 10),
    ]),
    { messageId: "9007199254740992", deliveredTime: "10" },
  );
});

Deno.test("履歴は境界を重複させず新しいキャッシュを残して次の位置を返す", async () => {
  const f = fixture([[
    raw("9007199254740995", 20),
    raw("9007199254740994", 20),
    raw("9007199254740993", 10),
  ]]);
  const cursor = { messageId: "9007199254740995", deliveredTime: "20" };
  await f.fetch(1, "c-test", cursor);
  assertEquals(f.failures, []);
  assertEquals(f.calls[0].endMessageId, {
    messageId: 9007199254740995n,
    deliveredTime: 20n,
  });
  assertEquals((f.results[0].messages as Json[]).map((m) => m.id), [
    "9007199254740993",
    "9007199254740994",
  ]);
  assertEquals(f.results[0].historyCursor, {
    messageId: "9007199254740993",
    deliveredTime: "10",
  });
  assertEquals(f.results[0].hasMore, true);
  assertEquals(f.saved[0].map((m) => m.id), [
    "9007199254740993",
    "9007199254740994",
    "9007199254741000",
  ]);
  assertEquals(f.runtime.msgCache.get("c-test")!.at, 123);
  assertEquals(f.media, [{ isolated: true }]);
});

for (const page of [[], [raw("10", 10)], [raw("11", 20)]]) {
  Deno.test(`空・境界だけ・新しい履歴だけの応答で追加取得を停止する: ${page.length ? page[0].id : "空"}`, async () => {
    const f = fixture([page]);
    await f.fetch(1, "c-test", { messageId: "10", deliveredTime: "10" });
    assertEquals(f.results[0].hasMore, false);
    assertEquals(f.results[0].messages, []);
  });
}

Deno.test("配信時刻のない既存キャッシュは最新の取得位置を補完してから遡る", async () => {
  const f = fixture([[raw("50", 50), raw("40", 40)], [raw("30", 30)]]);
  await f.fetch(1, "c-test", null);
  assertEquals(f.calls.length, 2);
  assertEquals(f.calls[1].endMessageId, { messageId: 40n, deliveredTime: 40n });
  assertEquals(f.results[0].historyCursor, {
    messageId: "30",
    deliveredTime: "30",
  });
  assertEquals(historyState([{ id: "10", createdTime: 10 }]), {
    historyCursor: null,
    hasMore: true,
  });
});

Deno.test("不正なカーソルでは通信せず、通信失敗後にも同じ位置から再試行できる", async () => {
  const f = fixture([[raw("5", 5)]]);
  await f.fetch(1, "c-test", { messageId: "bad", deliveredTime: "10" });
  assertEquals(f.calls, []);
  assertEquals(f.failures, ["invalid_history_cursor"]);
  const client = f.runtime.getClient()!;
  const original = client.base.talk.getPreviousMessagesV2WithRequest;
  client.base.talk.getPreviousMessagesV2WithRequest = () =>
    Promise.reject(new Error("offline"));
  const cursor = { messageId: "10", deliveredTime: "10" };
  await f.fetch(2, "c-test", cursor);
  assertEquals(f.failures.at(-1), "offline");
  assertEquals(f.saved, []);
  client.base.talk.getPreviousMessagesV2WithRequest = original;
  await f.fetch(3, "c-test", cursor);
  assertEquals(f.results[0].hasMore, true);
});

Deno.test("取得中にログアウトした履歴を保存・表示しない", async () => {
  const f = fixture([]);
  f.runtime.getClient()!.base.talk.getPreviousMessagesV2WithRequest = () => {
    f.logout();
    return Promise.resolve([raw("5", 5)] as never);
  };
  await f.fetch(1, "c-test", { messageId: "10", deliveredTime: "10" });
  assertEquals(f.failures, ["session_changed"]);
  assertEquals(f.results, []);
  assertEquals(f.saved, []);
});

Deno.test("保存済みグループイベントは取得した履歴の時間範囲だけに含める", async () => {
  const f = fixture([[raw("10", 10), raw("5", 5)]]);
  f.runtime.forUiMessages = (messages) =>
    Promise.resolve([
      ...messages,
      { id: "event-before", createdTime: 1, chatEvent: {} },
      { id: "event-in-page", createdTime: 5, chatEvent: {} },
      { id: "event-after", createdTime: 20, chatEvent: {} },
    ]);
  await f.fetch(1, "c-test", { messageId: "20", deliveredTime: "20" });
  assertEquals((f.results[0].messages as Json[]).map((m) => m.id), [
    "5",
    "10",
    "event-in-page",
  ]);
});
