import { assert, assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import {
  canReact,
  createReactions,
  normalizeReactions,
} from "../src/reactions.ts";
type Json = Record<string, unknown>;
const time = 1_800_000_000_000;
const original = (extra: Json = {}): Json => ({
  id: "9007199254740993",
  from: "u-peer",
  to: "u-self",
  mine: false,
  createdTime: time - 1000,
  contentType: "NONE",
  reactions: [],
  ...extra,
});
function fixture() {
  const calls: unknown[] = [], events: Json[] = [];
  let message: Json | undefined = original();
  let blocked = false, fail = false, clock = time;
  let sequence = 0;
  let gate: Promise<number> | undefined;
  let sendGate: Promise<void> | undefined;
  const client = {
    base: {
      profile: { mid: "u-self" },
      getReqseq: () => gate ?? Promise.resolve(++sequence),
      talk: {
        react: (value: unknown) => {
          calls.push(value);
          return fail
            ? Promise.reject(new Error("offline"))
            : sendGate ?? Promise.resolve();
        },
        cancelReaction: (value: unknown) => {
          calls.push(value);
          return fail
            ? Promise.reject(new Error("offline"))
            : Promise.resolve();
        },
      },
    },
  } as unknown as Client;
  let active: Client | null = client;
  const service = createReactions({
    getClient: () => active,
    findMessage: () => Promise.resolve(message),
    isBlocked: () => blocked,
    changed: () => {},
    observeUser: () => {},
    emit: (event, value) => events.push({ event, ...value }),
    now: () => clock,
  });
  const op = (kind: unknown, extra: Json = {}) =>
    service.handleOperation({
      type: 140,
      param1: "9007199254740993",
      param2: JSON.stringify({
        chatMid: "u-peer",
        curr: kind === null ? null : { predefinedReactionType: kind },
      }),
      param3: "u-peer",
      createdTime: clock,
      ...extra,
    });
  return {
    service,
    calls,
    events,
    op,
    active: (next: Client | null) => active = next,
    message: (next: Json | undefined) => message = next,
    blocked: () => blocked = true,
    fail: () => fail = true,
    clock: (next: number) => clock = next,
    gate: (next: Promise<number>) => gate = next,
    sendGate: (next: Promise<void>) => sendGate = next,
  };
}

Deno.test("履歴のリアクションを種類・人・時刻で正規化し、新形式を誤った既存アイコンにしない", () => {
  assertEquals(
    normalizeReactions([
      {
        fromUserMid: "u-a",
        atMillis: "10",
        reactionType: { predefinedReactionType: "LOVE" },
      },
      {
        fromUserMid: "u-a",
        atMillis: "9",
        reactionType: { predefinedReactionType: 2 },
      },
      {
        fromUserMid: "u-b",
        atMillis: 11,
        reactionType: { paidReactionType: { emojiId: "3" } },
      },
      { fromUserMid: "c-group", reactionType: { predefinedReactionType: 2 } },
    ]),
    [{ userMid: "u-a", kind: 3, at: 10 }, { userMid: "u-b", kind: 0, at: 11 }],
  );
});
Deno.test("追加・変更・取り消しを正確な64bit IDと別々の順序番号で送り、成功後にだけ反映する", async () => {
  const f = fixture();
  await f.service.change("u-peer", "9007199254740993", 2);
  await f.service.change("u-peer", "9007199254740993", 3);
  await f.service.change("u-peer", "9007199254740993", null);
  assertEquals(f.calls, [
    { id: 9007199254740993n, reaction: 2, reqSeq: 1 },
    { id: 9007199254740993n, reaction: 3, reqSeq: 2 },
    { cancelReactionRequest: { reqSeq: 3, messageId: 9007199254740993n } },
  ]);
  assertEquals(f.events.length, 3);
  assertEquals((f.events[2].reaction as Json).kind, -1);
  f.fail();
  await assertRejects(() => f.service.change("u-peer", "9007199254740993", 4));
  assertEquals(f.events.length, 3);
});
Deno.test("期限切れ・未送信・取消済み・別トーク・復号失敗・ブロックを送信前に拒否する", async () => {
  const f = fixture();
  for (
    const extra of [
      { createdTime: time - 7 * 86400_000 },
      { id: "pending-1" },
      { unsentRetained: true },
      { contentType: "UNSENT" },
      { contentType: "CALL" },
      { to: "c-other" },
      { decryptionFailed: true },
    ]
  ) {
    f.message(original(extra));
    await assertRejects(() =>
      f.service.change("u-peer", "9007199254740993", 2)
    );
  }
  f.message(original());
  f.blocked();
  await assertRejects(() => f.service.change("u-peer", "9007199254740993", 2));
  assertEquals(f.calls, []);
  assert(canReact(original({ to: "r-room" }), "r-room", time));
});
Deno.test("通知の重複・変更・取り消しを同じ人の差分にまとめ、古い履歴で新しい結果を戻さない", () => {
  const f = fixture();
  f.service.decorate("u-peer", [original()]);
  f.op(2, { revision: "100" });
  f.op(2, { revision: "100" });
  assertEquals(f.events.length, 1);
  f.op(3, { revision: "101" });
  f.op(6, { revision: "99" });
  let message = f.service.decorate("u-peer", [original()])[0];
  assertEquals((message.reactions as Json[]).map((r) => r.kind), [3]);
  f.op(null, { revision: "102" });
  message = f.service.decorate("u-peer", [
    original({ reactions: [{ userMid: "u-peer", kind: 3, at: time - 1 }] }),
  ])[0];
  assertEquals(message.reactions, []);
});
Deno.test("自分の別端末の操作は自分として反映し、新形式は種類不明・nullは取り消しにする", () => {
  const f = fixture();
  f.op(2, { type: "SEND_REACTION", param3: "u-other" });
  assertEquals((f.events[0].reaction as Json).userMid, "u-self");
  f.op(999);
  assertEquals((f.events[1].reaction as Json).kind, 0);
  f.op(null);
  assertEquals((f.events[2].reaction as Json).kind, -1);
  f.op(2, { param3: "u-unrelated" });
  f.op(2, { param1: "not-a-message" });
  assertEquals(f.events.length, 3);
});
Deno.test("送信待ちの連打・アカウント変更では別の操作を送らず、別アカウントへ通知を混ぜない", async () => {
  const f = fixture();
  const gate = Promise.withResolvers<number>();
  f.gate(gate.promise);
  const first = f.service.change("u-peer", "9007199254740993", 2);
  await assertRejects(() => f.service.change("u-peer", "9007199254740993", 3));
  f.active(null);
  gate.resolve(1);
  await assertRejects(() => first);
  assertEquals(f.calls, []);
  assertEquals(f.events, []);
  assertEquals(f.service.decorate("u-peer", [original()])[0].reactions, []);
});
Deno.test("最新のサーバー情報は古い通知より優先し、期限到達を送信直前にも確認する", async () => {
  const f = fixture();
  f.op(2);
  const message = f.service.decorate("u-peer", [
    original({ reactionUpdatedTime: time + 1, reactions: [] }),
  ])[0];
  assertEquals(message.reactions, []);
  const gate = Promise.withResolvers<number>();
  f.gate(gate.promise);
  const pending = f.service.change("u-peer", "9007199254740993", 2);
  f.clock(time + 8 * 86400_000);
  gate.resolve(1);
  await assertRejects(() => pending);
  assertEquals(f.calls, []);
});

Deno.test("順序番号を待つ間に本文が取り消されたらリアクションを送らない", async () => {
  const f = fixture();
  const gate = Promise.withResolvers<number>();
  f.gate(gate.promise);
  const pending = f.service.change("u-peer", "9007199254740993", 2);
  await new Promise((r) => setTimeout(r, 0));
  f.message(original({ contentType: "UNSENT" }));
  gate.resolve(1);
  await assertRejects(() => pending);
  assertEquals(f.calls, []);
});

Deno.test("時刻差があっても通知の順序番号を優先し、遅い成功応答で別端末の新しい操作を戻さない", async () => {
  const f = fixture();
  f.op(2, { type: 139, revision: "100", createdTime: time + 5000 });
  const gate = Promise.withResolvers<void>();
  f.sendGate(gate.promise);
  const pending = f.service.change("u-peer", "9007199254740993", 3);
  while (!f.calls.length) await new Promise((r) => setTimeout(r, 0));
  f.op(4, { type: 139, revision: "101", createdTime: time + 1000 });
  gate.resolve();
  await pending;
  f.op(2, { type: 139, revision: "100", createdTime: time + 5000 });
  const messages = f.service.decorate("u-peer", [original()]);
  assertEquals((messages[0].reactions as Json[]).map((r) => r.kind), [4]);
});
