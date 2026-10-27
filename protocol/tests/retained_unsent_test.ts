import { assert, assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createRetainedUnsent } from "../src/retained_unsent.ts";
import { createUnsendService } from "../src/unsend.ts";
type Json = Record<string, unknown>;
const message = (id = "100", time = 100): Json => ({
  id,
  from: "u-peer",
  to: "u-self",
  mine: false,
  text: "受信した本文\n🙂",
  createdTime: time,
  contentType: "NONE",
});
function fixture(dataDir: string) {
  let account = "u-self";
  let disk: Json | undefined;
  let diskRead: () => Promise<Json | undefined> = () => Promise.resolve(disk);
  const events: Json[] = [];
  const calls: unknown[] = [];
  const runtime = {
    dataDir,
    selfMid: () => account,
    peekDisk: () => diskRead(),
  };
  const keep = createRetainedUnsent(runtime);
  const unsendRuntime = {
    dataDir,
    retained: keep,
    getClient: () =>
      ({
        base: {
          profile: { mid: account },
          getReqseq: () => Promise.resolve(7),
          talk: {
            unsendMessage: (args: unknown) => {
              calls.push(args);
              return Promise.resolve();
            },
          },
        },
      }) as unknown as Client,
    findMessage: () =>
      Promise.resolve({
        ...message("200"),
        from: account,
        to: "u-peer",
        mine: true,
      }),
    changed: () => Promise.resolve(),
    emit: (event: string, data: Json) => events.push({ event, ...data }),
  };
  const service = createUnsendService(unsendRuntime);
  return {
    runtime,
    keep,
    service,
    unsendRuntime,
    events,
    calls,
    account: (value: string) => account = value,
    disk: (value: Json | undefined) => disk = value,
    diskRead: (value: typeof diskRead) => diskRead = value,
    cancel: async (id = "100", chat = "u-peer") => {
      assert(service.handleOperation({ type: 65, param1: chat, param2: id }));
      await service.flush();
    },
  };
}

Deno.test("受信済み本文を取消の印付きで残し、再起動とサーバーから消えた履歴へ復元する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    const original = message();
    original.replyTo = {
      messageId: "99",
      text: "保存コピーへ複製しない引用文",
      mine: true,
    };
    f.keep.observe("u-peer", [original]);
    await f.cancel();
    await f.cancel();
    assertEquals(f.events.length, 1);
    assertEquals(f.calls, []);
    const saved = (await f.service.mask("u-peer", [original]))[0];
    assertEquals(saved.text, original.text);
    assertEquals(saved.unsentRetained, true);
    assertEquals(saved.unsentAccountMid, "u-self");
    assertEquals(saved.replyTo, { messageId: "99" });
    assertEquals(
      (await Deno.readTextFile(dir + "/retained-unsent-u-self.json"))
        .includes("保存コピーへ複製しない引用文"),
      false,
    );
    assertEquals((f.events[0].retainedMessage as Json).text, original.text);
    const restarted = createRetainedUnsent(f.runtime);
    assertEquals(
      (await restarted.get("u-self", "u-peer", "100"))?.text,
      original.text,
    );
    const restored = await restarted.merge("u-self", "u-peer", [
      message("99", 99),
    ], true);
    assertEquals(restored.map((m) => m.id), ["99", "100"]);
    assertEquals(
      (await restarted.merge("u-self", "u-peer", [], true))[0].id,
      "100",
    );
    if (Deno.build.os !== "windows") {
      assertEquals(
        (await Deno.stat(dir + "/retained-unsent-u-self.json")).mode! & 0o777,
        0o600,
      );
    }
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("自分の取消は本文を残さず、他アカウントと別のトークへ内容を渡さない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.keep.observe("u-peer", [message()]);
    await f.cancel();
    assertEquals(await f.keep.get("u-other", "u-peer", "100"), undefined);
    assertEquals(await f.keep.get("u-self", "u-other", "100"), undefined);
    f.account("u-other");
    const foreign = (await f.service.mask("u-peer", [{
      ...message(),
      unsentRetained: true,
      unsentAccountMid: "u-self",
    }]))[0];
    assertEquals(foreign.contentType, "UNSENT");
    assertEquals(foreign.text, "");
    f.account("u-self");
    await f.service.unsend("u-peer", "200", 1);
    const own = (await f.service.mask("u-peer", [{
      ...message("200"),
      from: "u-self",
      to: "u-peer",
      mine: true,
    }]))[0];
    assertEquals(own.contentType, "UNSENT");
    assertEquals(own.text, "");
    assertEquals(f.calls.length, 1);
    assertEquals(await f.keep.get("u-self", "u-peer", "200"), undefined);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("未受信・復号失敗・旧所有者不明の内容や、取消後に遅れて届いた本文は保存しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.cancel();
    f.keep.observe("u-peer", [message()]);
    await f.cancel();
    assertEquals(
      (await f.service.mask("u-peer", [message()]))[0].contentType,
      "UNSENT",
    );
    for (
      const [id, extra] of [["101", { decryptionFailed: true }], ["102", {
        retentionEligible: false,
      }], ["103", { mine: true }]] as const
    ) {
      f.keep.observe("u-peer", [{ ...message(id), ...extra }]);
      await f.cancel(id);
      assertEquals(await f.keep.get("u-self", "u-peer", id), undefined);
    }
    assertEquals(f.calls, []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("保存済みのメディアを保持し、新たな添付取得は要求しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.keep.observe("u-peer", [{
      ...message(),
      contentType: "IMAGE",
      imagePath: "/cached/received.png",
      imageUrl: "https://example.invalid/file",
      needsMedia: true,
    }]);
    await f.cancel();
    const retained = (await f.service.mask("u-peer", [message()]))[0];
    assertEquals(retained.imagePath, "/cached/received.png");
    assertEquals(retained.needsMedia, false);
    assertEquals(retained.imageUrl, null);
    assertEquals(f.calls, []);
    const old = await f.keep.merge("u-self", "u-peer", [
      message("1", 1),
      message("2", 2),
    ], false);
    assertEquals(old.map((m) => m.id), ["1", "2"]);
    assertEquals(await f.keep.merge("u-self", "u-peer", [], false), []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("保存中のアカウント変更で新しい画面へ本文を流用しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    const gate = Promise.withResolvers<Json | undefined>();
    f.diskRead(() => gate.promise);
    f.service.handleOperation({ type: 65, param1: "u-peer", param2: "100" });
    await new Promise((r) => setTimeout(r, 20));
    f.account("u-other");
    gate.resolve(message());
    await f.service.flush();
    assertEquals(f.events, []);
    assertEquals(await f.keep.get("u-other", "u-peer", "100"), undefined);
    assertEquals(await f.keep.get("u-self", "u-peer", "100"), undefined);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("キャッシュ削除は保存本文を消し、取消済みIDから元の本文が復活しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.keep.observe("u-peer", [message()]);
    await f.cancel();
    await f.keep.clear();
    assertEquals(await f.keep.get("u-self", "u-peer", "100"), undefined);
    assertEquals(
      (await f.service.mask("u-peer", [message()]))[0].contentType,
      "UNSENT",
    );
    await assertRejects(
      () => Deno.stat(dir + "/retained-unsent-u-self.json"),
      Deno.errors.NotFound,
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("壊れた保存ファイルを初期化せず、元ファイルを維持する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const file = dir + "/retained-unsent-u-self.json";
    await Deno.writeTextFile(file, "broken");
    const f = fixture(dir);
    f.keep.observe("u-peer", [message()]);
    await f.cancel();
    assertEquals(await Deno.readTextFile(file), "broken");
    assertEquals(
      (await f.service.mask("u-peer", [message()]))[0].contentType,
      "UNSENT",
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("グループの受信済みスタンプも元のトークとアカウントにだけ戻す", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.keep.observe("c-group", [{
      ...message(),
      to: "c-group",
      contentType: "STICKER",
      stickerId: "123",
      imagePath: "/cached/sticker.png",
    }]);
    await f.cancel("100", "c-group");
    const kept = (await f.keep.merge("u-self", "c-group", [], true))[0];
    assertEquals(kept.contentType, "STICKER");
    assertEquals(kept.stickerId, "123");
    assertEquals(kept.imagePath, "/cached/sticker.png");
    assertEquals(await f.keep.merge("u-other", "c-group", [], true), []);
    assertEquals(await f.keep.merge("u-self", "c-other", [], true), []);
    assertEquals(f.calls, []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
