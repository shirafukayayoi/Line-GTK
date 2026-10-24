import { assertEquals, assertRejects } from "@std/assert";
import {
  createScheduledSender,
  guardedScheduledTalk,
  type ScheduledText,
} from "../src/scheduled_send.ts";
import type { Client } from "@evex/linejs";

const input = (): ScheduledText => ({
  accountMid: "u-self",
  reservationId: "1",
  chatMid: "c-group",
  text: "予約した返信\n改行🙂",
  sendAt: 3000,
  replyToMessageId: "9007199254740993",
});

function fixture() {
  const state = {
    now: 3001,
    account: "u-self",
    ready: true,
    blocked: false,
    calls: [] as ScheduledText[],
    fail: false,
  };
  const send = createScheduledSender({
    now: () => state.now,
    account: () => state.account,
    ready: () => state.ready,
    blocked: () => state.blocked,
    send: (params) => {
      state.calls.push(params);
      if (state.fail) return Promise.reject(new Error("通信中に切断"));
      return Promise.resolve({ message: { id: "42" } });
    },
  });
  return { state, send };
}

Deno.test("予約の宛先・改行・64bit引用IDを維持して一度だけ送信する", async () => {
  const { state, send } = fixture();
  assertEquals(await send(input()), { message: { id: "42" } });
  assertEquals(state.calls, [input()]);
  await assertRejects(() => send(input()), Error, "scheduled_duplicate");
  assertEquals(state.calls.length, 1);
});

Deno.test("送信時刻より前・期限切れ・別アカウント・未接続・ブロックでは送らない", async () => {
  for (
    const override of [
      { now: 2999 },
      { now: 5001 },
      { account: "u-other" },
      { ready: false },
      { blocked: true },
    ]
  ) {
    const { state, send } = fixture();
    Object.assign(state, override);
    await assertRejects(() => send(input()));
    assertEquals(state.calls.length, 0);
  }
});

Deno.test("通信エラー後も同じ予約を別経路へ自動再送しない", async () => {
  const { state, send } = fixture();
  state.fail = true;
  await assertRejects(() => send(input()), Error, "scheduled_send_unknown");
  state.fail = false;
  await assertRejects(() => send(input()), Error, "scheduled_duplicate");
  assertEquals(state.calls.length, 1);
});

Deno.test("同時の重複要求も送信開始前に排除する", async () => {
  const { state, send } = fixture();
  const results = await Promise.allSettled([send(input()), send(input())]);
  assertEquals(
    results.filter((result) => result.status === "fulfilled").length,
    1,
  );
  assertEquals(state.calls.length, 1);
});

Deno.test("不正な予約と引用IDを拒否し、引用なしでも送れる", async () => {
  for (
    const override of [
      { text: " \n" },
      { text: "あ".repeat(10_001) },
      { sendAt: "3000" },
      { replyToMessageId: "pending-1" },
      { reservationId: "" },
      { chatMid: "other" },
    ]
  ) {
    const { state, send } = fixture();
    await assertRejects(() => send({ ...input(), ...override }));
    assertEquals(state.calls.length, 0);
  }
  const { state, send } = fixture();
  await send({ ...input(), replyToMessageId: null });
  assertEquals(state.calls[0].replyToMessageId, undefined);
});

type MockBase = {
  request: MockRequest;
  getReqseq: () => Promise<number>;
  fetch: (request: Request) => Promise<Response>;
  talk: { client: MockBase; sendMessage: () => Promise<Response> };
};
class MockRequest {
  #client: MockBase;
  constructor(client: MockBase) {
    this.#client = client;
  }
  async transmit() {
    await Promise.resolve();
    return await this.#client.fetch(
      new Request("https://example.invalid/scheduled-fixture"),
    );
  }
}

Deno.test("送信準備のawait後と実通信直前でも期限・アカウント・ブロックを確認する", async () => {
  for (
    const mode of [
      "ready",
      "late",
      "account",
      "blocked",
      "session",
      "transport-late",
    ]
  ) {
    const due = Date.now();
    let now = due;
    let account = "u-self";
    let blocked = false;
    let owner: object = {};
    let calls = 0;
    const base: MockBase = {
      request: null as unknown as MockRequest,
      getReqseq: async () => {
        await Promise.resolve();
        if (mode === "late") now += 3000;
        if (mode === "account") account = "u-other";
        if (mode === "blocked") blocked = true;
        if (mode === "session") owner = {};
        return 1;
      },
      fetch: () => {
        calls++;
        return Promise.reject(new Error("模擬ネットワーク"));
      },
      talk: {
        client: null as unknown as MockBase,
        async sendMessage() {
          await this.client.getReqseq();
          if (mode === "transport-late") now += 3000;
          return await this.client.request.transmit();
        },
      },
    };
    base.request = new MockRequest(base);
    base.talk.client = base;
    const original = base.request;
    const send = createScheduledSender({
      now: () => now,
      account: () => account,
      session: () => owner,
      ready: () => true,
      blocked: () => blocked,
      send: (input, check) =>
        guardedScheduledTalk(
          base as unknown as Client["base"],
          input.sendAt,
          check,
        ).sendMessage({ to: input.chatMid, text: input.text, e2ee: true }),
    });
    await assertRejects(
      () => send({ ...input(), sendAt: due }),
      Error,
      mode === "ready"
        ? "scheduled_send_unknown"
        : mode === "late" || mode === "transport-late"
        ? "scheduled_expired"
        : mode === "blocked"
        ? "contact_blocked_send"
        : "scheduled_unavailable",
    );
    assertEquals(calls, mode === "ready" ? 1 : 0);
    assertEquals(base.request, original);
    assertEquals(base.talk.client, base);
    // 同じクライアントを使う通常の通信へ予約の制約を持ち出さない。
    await assertRejects(
      () => base.talk.sendMessage(),
      Error,
      "模擬ネットワーク",
    );
    assertEquals(calls, mode === "ready" ? 2 : 1);
  }
});
