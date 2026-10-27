import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";
import { replyMessageId } from "./replies.ts";

type Json = Record<string, unknown>;
type Entry = { chatMid: string; message: Json };
type Data = { version: number; accountMid: string; messages: Entry[] };
type Runtime = {
  dataDir: string;
  selfMid: () => string;
  peekDisk: (
    account: string,
    chat: string,
    id: string,
  ) => Promise<Json | undefined>;
};
const MAX_BYTES = 8 * 1024 * 1024;
const MAX_MESSAGES = 1000;
const accountId = (value: string) => /^u[a-zA-Z0-9_-]{1,128}$/.test(value);
const chatId = (value: string) => /^[ucr][a-zA-Z0-9_-]{1,128}$/.test(value);
function received(message: Json, account: string, chat: string): boolean {
  const from = String(message.from ?? "");
  const to = String(message.to ?? "");
  return accountId(account) && chatId(chat) && accountId(from) &&
    from !== account &&
    message.mine === false && !!replyMessageId(message.id) &&
    typeof message.text === "string" &&
    typeof message.contentType === "string" &&
    typeof message.createdTime === "number" &&
    Number.isFinite(message.createdTime) &&
    message.retentionEligible !== false && message.decryptionFailed !== true &&
    !message.chatEvent &&
    !["UNSENT", "CALL", "CHAT_EVENT"].includes(
      String(message.contentType).toUpperCase(),
    ) &&
    (/^[cr]/.test(chat) ? to === chat : from === chat && to === account);
}

export function createRetainedUnsent(runtime: Runtime) {
  const loaded = new Map<string, Promise<Data>>();
  const observed = new Map<string, { message: Json; bytes: number }>();
  let observedBytes = 0;
  let owner = "";
  let writes: Promise<unknown> = Promise.resolve();
  const path = (account: string) =>
    join(runtime.dataDir, `retained-unsent-${account}.json`);
  function sync() {
    const account = runtime.selfMid();
    if (owner !== account) {
      owner = account;
      observed.clear();
      observedBytes = 0;
    }
    return account;
  }
  async function load(account: string): Promise<Data> {
    if (!accountId(account)) throw new Error("retained_unsent_invalid_account");
    let task = loaded.get(account);
    if (!task) {
      task = (async () => {
        try {
          const file = path(account);
          if ((await Deno.stat(file)).size > MAX_BYTES) {
            throw new Error("retained_unsent_too_large");
          }
          const data = JSON.parse(await Deno.readTextFile(file)) as Data;
          const seen = new Set<string>();
          if (
            data.version !== 1 || data.accountMid !== account ||
            !Array.isArray(data.messages) ||
            data.messages.length > MAX_MESSAGES ||
            data.messages.some((entry) => {
              if (
                !entry || !entry.message || typeof entry.message !== "object" ||
                typeof entry.chatMid !== "string" ||
                !received(entry.message, account, entry.chatMid)
              ) return true;
              const key = `${entry.chatMid}:${entry.message.id}`;
              if (seen.has(key)) return true;
              seen.add(key);
              return false;
            })
          ) throw new Error("retained_unsent_invalid_store");
          return data;
        } catch (error) {
          if (error instanceof Deno.errors.NotFound) {
            return { version: 1, accountMid: account, messages: [] };
          }
          throw error;
        }
      })();
      loaded.set(account, task);
    }
    return await task;
  }

  function observe(chat: string | undefined, messages: Json[]) {
    const account = sync();
    if (!accountId(account)) return;
    for (const message of messages) {
      const mid = chat ??
        (/^[cr]/.test(String(message.to))
          ? String(message.to)
          : String(message.from));
      if (!received(message, account, mid) || message.unsentRetained === true) {
        continue;
      }
      const raw = JSON.stringify(message);
      const bytes = new TextEncoder().encode(raw).length;
      if (bytes > 1024 * 1024) continue;
      const key = `${mid}:${message.id}`;
      observedBytes -= observed.get(key)?.bytes ?? 0;
      observed.delete(key);
      observed.set(key, { message: JSON.parse(raw), bytes });
      observedBytes += bytes;
      while (observed.size > 2000 || observedBytes > MAX_BYTES) {
        const first = observed.keys().next().value!;
        observedBytes -= observed.get(first)!.bytes;
        observed.delete(first);
      }
    }
  }

  function capture(
    account: string,
    chat: string,
    id: string,
  ): Promise<Json | undefined> {
    const task = writes.catch(() => {}).then(async () => {
      if (sync() !== account || !chatId(chat) || !replyMessageId(id)) return;
      const data = await load(account);
      const previous = data.messages.find((entry) =>
        entry.chatMid === chat && String(entry.message.id) === id
      );
      if (previous) return visible(previous.message, account);
      const message = observed.get(`${chat}:${id}`)?.message ??
        await runtime.peekDisk(account, chat, id);
      if (
        runtime.selfMid() !== account || !message ||
        String(message.id) !== id || !received(message, account, chat)
      ) return;
      const raw = JSON.stringify(message);
      if (new TextEncoder().encode(raw).length > 1024 * 1024) return;
      const snapshot = JSON.parse(raw) as Json;
      delete snapshot.imageUrl;
      // 引用元の本文は複製せず、通常の履歴からIDで補完する。
      if (snapshot.replyTo && typeof snapshot.replyTo === "object") {
        const quote = snapshot.replyTo as Json;
        snapshot.replyTo = replyMessageId(quote.messageId)
          ? { messageId: String(quote.messageId) }
          : null;
      }
      const next: Data = {
        version: 1,
        accountMid: account,
        messages: [...data.messages, { chatMid: chat, message: snapshot }]
          .slice(-MAX_MESSAGES),
      };
      while (
        new TextEncoder().encode(JSON.stringify(next)).length > MAX_BYTES
      ) next.messages.shift();
      await atomicWriteJson(path(account), next);
      loaded.set(account, Promise.resolve(next));
      return visible(snapshot, account);
    });
    writes = task;
    return task;
  }

  function visible(message: Json, account: string): Json {
    return {
      ...structuredClone(message),
      unsentRetained: true,
      unsentAccountMid: account,
      needsMedia: false,
      imageUrl: null,
    };
  }

  async function get(
    account: string,
    chat: string,
    id: string,
  ): Promise<Json | undefined> {
    try {
      const data = await load(account);
      const entry = data.messages.find((entry) =>
        entry.chatMid === chat && String(entry.message.id) === id
      );
      return entry ? visible(entry.message, account) : undefined;
    } catch {
      console.error(
        "[retained-unsent] 保存済みの受信内容を読み込めませんでした。",
      );
      return undefined;
    }
  }

  async function merge(
    account: string,
    chat: string,
    messages: Json[],
    latest: boolean,
  ): Promise<Json[]> {
    if (!accountId(account)) return messages;
    try {
      const data = await load(account);
      const times = messages.map((m) => Number(m.createdTime)).filter((n) =>
        Number.isFinite(n) && n > 0
      );
      if (!latest && !times.length) return messages;
      const lower = times.length ? Math.min(...times) : 0;
      const upper = times.length ? Math.max(...times) : Infinity;
      const retained = data.messages.filter((entry) =>
        entry.chatMid === chat &&
        Number(entry.message.createdTime) >= lower &&
        (latest || Number(entry.message.createdTime) <= upper)
      );
      const merged = new Map(
        messages.map((message) => [String(message.id), message]),
      );
      for (const entry of retained) {
        merged.set(String(entry.message.id), visible(entry.message, account));
      }
      return [...merged.values()].sort((a, b) =>
        Number(a.createdTime) - Number(b.createdTime)
      );
    } catch {
      return messages;
    }
  }

  async function clear() {
    const task = writes.catch(() => {}).then(async () => {
      observed.clear();
      observedBytes = 0;
      loaded.clear();
      for await (const entry of Deno.readDir(runtime.dataDir)) {
        if (
          entry.isFile &&
          /^retained-unsent-u[a-zA-Z0-9_-]{1,128}\.json$/.test(entry.name)
        ) {
          await Deno.remove(join(runtime.dataDir, entry.name));
        }
      }
    });
    writes = task;
    await task;
  }
  return { observe, capture, get, merge, clear };
}
