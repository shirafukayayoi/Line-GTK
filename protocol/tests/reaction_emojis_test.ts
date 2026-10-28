import { assert, assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createReactionCatalog } from "../src/reaction_catalog.ts";
import {
  emojiKey,
  paidReaction,
  sendEmojiReaction,
} from "../src/reaction_emojis.ts";
import { createReactions, normalizeReactions } from "../src/reactions.ts";

const product = "670e0cce840a8236ddd4ee4c";
const owned = "1234567890abcdef12345678";
const emoji = {
  productId: product,
  emojiId: "001",
  resourceType: 2,
  version: 123,
};

Deno.test("LINE絵文字のID・バージョン・資源種別を履歴と通知から保持し、別の絵文字をまとめない", () => {
  assertEquals(paidReaction({ 1: product, 2: "001", 3: 2, 4: "123" }), emoji);
  assertEquals(paidReaction({ ...emoji, emojiId: "../001" }), null);
  assertEquals(paidReaction({ ...emoji, version: -1 }), null);
  const normalized = normalizeReactions([
    {
      fromUserMid: "u-a",
      atMillis: 10,
      reactionType: { 2: { 1: product, 2: "001", 3: 2, 4: 123 } },
    },
    {
      fromUserMid: "u-b",
      atMillis: 11,
      reactionType: { paidReactionType: { ...emoji, emojiId: "002" } },
    },
  ]);
  assertEquals(normalized[0].emoji, emoji);
  assertEquals(emojiKey(normalized[1].emoji!), product + "_002");
});

Deno.test("LINE絵文字のThrift送信はunionの2番とi64のバージョンを使い、既存の6種類へ変換しない", async () => {
  const calls: unknown[][] = [];
  const client = {
    base: {
      talk: { protocolType: 4, requestPath: "/S4" },
      request: {
        request: (...args: unknown[]) => {
          calls.push(args);
          return Promise.resolve();
        },
      },
    },
  } as unknown as Client;
  await sendEmojiReaction(client, 42, "9007199254740993", emoji);
  assertEquals(calls, [[
    [[12, 1, [
      [8, 1, 42],
      [10, 2, 9007199254740993n],
      [12, 3, [[12, 2, [
        [11, 1, product],
        [11, 2, "001"],
        [8, 3, 2],
        [10, 4, 123n],
      ]]]],
    ]]],
    "react",
    4,
    true,
    "/S4",
  ]]);
});

function catalogue() {
  const calls: unknown[][] = [], images: string[][] = [];
  let rows: unknown[] = [{
    1: owned,
    11: "保有絵文字",
    21: "42",
    93: { 3: { 3: "ANIMATION" } },
  }];
  let fail = false;
  const client = {
    base: {
      profile: { mid: "u-self" },
      request: {
        request: (...args: unknown[]) => {
          calls.push(args);
          return fail
            ? Promise.reject(new Error("offline"))
            : Promise.resolve({ 1: rows, 3: rows.length });
        },
      },
    },
  } as unknown as Client;
  let active: Client | null = client;
  let metadata: unknown = {
    productId: owned,
    orders: ["001", "002", "001", "../003"],
    altTexts: { "001": "最初の絵文字" },
  };
  const service = createReactionCatalog({
    getClient: () => active,
    images: (keys) => images.push(keys),
    fetch: (() =>
      Promise.resolve(new Response(JSON.stringify(metadata)))) as typeof fetch,
  });
  return {
    client,
    service,
    calls,
    images,
    rows: (value: unknown[]) => rows = value,
    active: (value: Client | null) => active = value,
    metadata: (value: unknown) => metadata = value,
    fail: () => fail = true,
  };
}

Deno.test("保有パッケージと公開メタデータから一覧を作り、送信には正規のバージョン・種別を使う", async () => {
  const f = catalogue();
  const list = await f.service.list();
  assertEquals(list.packages.map((p) => p.id), [product, owned]);
  const choices = await f.service.pack(owned);
  assertEquals(choices.length, 2);
  assertEquals(choices[0].label, "最初の絵文字");
  assertEquals(choices[0].emoji, {
    productId: owned,
    emojiId: "001",
    version: 42,
    resourceType: 2,
  });
  assertEquals(f.images, [[owned + "_001", owned + "_002"]]);
  assertEquals(
    (await f.service.resolve({
      ...emoji,
      productId: owned,
      version: 999,
      resourceType: 1,
    })).version,
    42,
  );
  assertEquals(f.calls.length, 1);
  await assertRejects(() => f.service.pack("abcdefabcdefabcdefabcdef"));
  await assertRejects(() =>
    f.service.resolve({ ...emoji, productId: owned, emojiId: "999" })
  );
  assertEquals(f.calls.length, 1);
});

Deno.test("取得失敗は標準パッケージと警告を残し、異なるメタデータは使わず再試行できる", async () => {
  const f = catalogue();
  f.fail();
  const list = await f.service.list(true);
  assertEquals(list.warning, true);
  assertEquals(list.packages.length, 1);
  await assertRejects(() => f.service.pack(product));
  f.metadata({ productId: product, orders: ["001"], altTexts: {} });
  assertEquals((await f.service.pack(product))[0].emoji.version, 1);
});

Deno.test("別アカウントへ切り替えた後は古い保有一覧を使わない", async () => {
  const f = catalogue();
  await f.service.list();
  f.active(null);
  await assertRejects(() => f.service.pack(owned));
  f.active(f.client);
  f.rows([]);
  await assertRejects(() => f.service.pack(owned));
  assertEquals(f.calls.length, 2);
});

Deno.test("LINE絵文字の送信成功・通知・取り消しを反映し、保有確認失敗時は送信しない", async () => {
  const sent: unknown[][] = [], events: Record<string, unknown>[] = [];
  const client = {
    base: {
      profile: { mid: "u-self" },
      getReqseq: () => Promise.resolve(3),
      talk: {
        protocolType: 4,
        requestPath: "/S4",
        cancelReaction: () => Promise.resolve(),
      },
      request: {
        request: (...args: unknown[]) => {
          sent.push(args);
          return Promise.resolve();
        },
      },
    },
  } as unknown as Client;
  let allowed = true;
  const service = createReactions({
    getClient: () => client,
    findMessage: () =>
      Promise.resolve({
        id: "100",
        from: "u-peer",
        to: "u-self",
        mine: false,
        createdTime: Date.now(),
        contentType: "NONE",
      }),
    isBlocked: () => false,
    changed: () => {},
    observeUser: () => {},
    emit: (event, data) => events.push({ event, ...data }),
    resolveEmoji: () =>
      allowed ? Promise.resolve(emoji) : Promise.reject(new Error("not_owned")),
  });
  await service.change("u-peer", "100", { ...emoji, version: 999 });
  assertEquals((events[0].reaction as Record<string, unknown>).emoji, emoji);
  assertEquals(sent.length, 1);
  assert(
    service.handleOperation({
      type: 140,
      param1: "100",
      param2: JSON.stringify({
        chatMid: "u-peer",
        curr: { paidReactionType: { ...emoji, emojiId: "002" } },
      }),
      param3: "u-peer",
    }),
  );
  assertEquals(
    ((events[1].reaction as Record<string, unknown>).emoji as typeof emoji)
      .emojiId,
    "002",
  );
  await service.change("u-peer", "100", null);
  assertEquals((events[2].reaction as Record<string, unknown>).kind, -1);
  allowed = false;
  await assertRejects(() => service.change("u-peer", "100", emoji));
  assertEquals(sent.length, 1);
});
