import type { Client } from "@evex/linejs";
import { conversationMid } from "./messages.ts";
import { replyMessageId } from "./replies.ts";

type Json = Record<string, unknown>;
export type Reaction = { userMid: string; kind: number; at: number };
type Operation = {
  type?: string | number;
  param1?: string;
  param2?: string;
  param3?: string;
  revision?: unknown;
  createdTime?: unknown;
};
type Patch = { reaction: Reaction; revision: bigint; expires: number };
type Runtime = {
  getClient: () => Client | null;
  findMessage: (chat: string, id: string) => Promise<Json | undefined>;
  isBlocked: (chat: string) => boolean;
  emit: (event: string, payload: Json) => void;
  changed: (
    chat: string,
    id: string,
    reaction: Reaction,
    version: number,
  ) => void;
  observeUser: (mid: string) => void;
  now?: () => number;
};
const record = (value: unknown): Json =>
  value && typeof value === "object" ? value as Json : {};
const user = (value: unknown): value is string =>
  typeof value === "string" && /^u[a-zA-Z0-9_-]{1,128}$/.test(value);
const chatId = (value: string) => /^[ucr][a-zA-Z0-9_-]{1,128}$/.test(value);
const kinds: Record<string, number> = {
  NICE: 2,
  LOVE: 3,
  FUN: 4,
  AMAZING: 5,
  SAD: 6,
  OMG: 7,
};
function kind(value: unknown): number {
  const number = typeof value === "string"
    ? kinds[value] ?? Number(value)
    : value;
  return typeof number === "number" && Number.isInteger(number) &&
      number >= 2 && number <= 7
    ? number
    : 0;
}
function millis(value: unknown): number {
  const number = Number(value);
  return Number.isSafeInteger(number) && number > 0 ? number : 0;
}

// 履歴は1人1件にまとめ、未対応の新形式は種類不明として区別する。
export function normalizeReactions(value: unknown): Reaction[] {
  const users = new Map<string, Reaction>();
  const values = Array.isArray(value) ? value : Object.values(record(value));
  for (const input of values.slice(0, 5000)) {
    const raw = record(input);
    const mid = raw.fromUserMid ?? raw.userMid;
    if (!user(mid)) continue;
    const type = record(raw.reactionType);
    const reaction = {
      userMid: mid,
      kind: kind(raw.kind ?? type.predefinedReactionType),
      at: millis(raw.atMillis ?? raw.at),
    };
    const previous = users.get(mid);
    if (!previous || reaction.at >= previous.at) users.set(mid, reaction);
  }
  return [...users.values()].sort((a, b) => a.userMid.localeCompare(b.userMid));
}

export function canReact(message: Json, chat: string, now: number): boolean {
  const time = millis(message.createdTime);
  return chatId(chat) && conversationMid(message) === chat &&
    !!replyMessageId(message.id) && time > 0 && time <= now + 60_000 &&
    now - time < 7 * 24 * 60 * 60 * 1000 &&
    !message.chatEvent && message.unsentRetained !== true &&
    message.decryptionFailed !== true &&
    !["CALL", "CHAT_EVENT", "UNSENT"].includes(
      String(message.contentType).toUpperCase(),
    );
}

export function createReactions(runtime: Runtime) {
  const now = runtime.now ?? Date.now;
  let client: Client | null = null;
  let account = "";
  let clock = 0;
  const patches = new Map<string, Map<string, Patch>>();
  const known = new Map<
    string,
    { reactions: Reaction[]; version: number; message?: Json }
  >();
  const pending = new Map<string, Promise<Json>>();
  const version = () => clock = Math.max(clock + 1, now() * 1000);
  function sync() {
    const next = runtime.getClient();
    const mid = String(next?.base.profile?.mid ?? "");
    if (next !== client || mid !== account) {
      client = next;
      account = mid;
      patches.clear();
      known.clear();
    }
    return next && user(mid) ? next : null;
  }
  function trim() {
    while (known.size > 2000) known.delete(known.keys().next().value!);
    while (patches.size > 2000) patches.delete(patches.keys().next().value!);
  }
  function decorate(chat: string, messages: Json[]): Json[] {
    if (!sync()) return messages;
    return messages.map((message) => {
      const key = `${chat}:${message.id}`;
      const reactions = new Map(
        normalizeReactions(message.reactions).map((r) => [r.userMid, r]),
      );
      const updates = patches.get(key);
      for (const [mid, patch] of updates ?? []) {
        const server = reactions.get(mid);
        if (
          patch.expires < now() ||
          Number(message.reactionUpdatedTime ?? 0) > patch.reaction.at ||
          (server?.at ?? 0) > patch.reaction.at
        ) {
          updates!.delete(mid);
          continue;
        }
        if (patch.reaction.kind === -1) reactions.delete(mid);
        else reactions.set(mid, patch.reaction);
      }
      if (message.contentType === "UNSENT" || message.unsentRetained === true) {
        reactions.clear();
      }
      const data = {
        reactions: [...reactions.values()],
        version: version(),
        message: {
          id: message.id,
          from: message.from,
          to: message.to,
          mine: message.mine,
          createdTime: message.createdTime,
          contentType: message.contentType,
          unsentRetained: message.unsentRetained,
          chatEvent: message.chatEvent,
          decryptionFailed: message.decryptionFailed,
        },
      };
      known.delete(key);
      known.set(key, data);
      trim();
      for (const r of data.reactions) runtime.observeUser(r.userMid);
      return {
        ...message,
        reactions: data.reactions,
        reactionVersion: data.version,
      };
    });
  }
  function apply(chat: string, id: string, reaction: Reaction, revision = 0n) {
    const key = `${chat}:${id}`;
    let updates = patches.get(key);
    if (!updates) {
      updates = new Map();
      patches.set(key, updates);
    }
    const previous = updates.get(reaction.userMid);
    if (
      previous &&
      (revision > 0n
        ? previous.revision >= revision
        : reaction.at > 0 && previous.reaction.at > reaction.at)
    ) return;
    updates.set(reaction.userMid, {
      reaction,
      revision: revision || previous?.revision || 0n,
      expires: now() + 600_000,
    });
    const current = known.get(key);
    const all = new Map((current?.reactions ?? []).map((r) => [r.userMid, r]));
    if (reaction.kind === -1) all.delete(reaction.userMid);
    else all.set(reaction.userMid, reaction);
    const data = {
      reactions: [...all.values()],
      version: version(),
      message: current?.message,
    };
    known.set(key, data);
    trim();
    runtime.observeUser(reaction.userMid);
    // 全履歴を再描画せず、操作した人の差分だけを画面へ届ける。
    runtime.emit("reaction", {
      accountMid: account,
      chatMid: chat,
      messageId: id,
      reaction,
      version: data.version,
    });
    runtime.changed(chat, id, reaction, data.version);
  }
  function handleOperation(op: Operation): boolean {
    const type = String(op.type);
    if (
      !["139", "140", "SEND_REACTION", "NOTIFIED_SEND_REACTION"].includes(type)
    ) return false;
    if (!sync()) return true;
    try {
      if (!op.param2 || op.param2.length > 16_384) return true;
      const payload = record(JSON.parse(op.param2));
      const chat = String(payload.chatMid ?? "");
      const id = replyMessageId(op.param1);
      const mid = type === "139" || type === "SEND_REACTION"
        ? account
        : op.param3;
      if (!chatId(chat) || !id || !user(mid)) return true;
      if (chat.startsWith("u") && mid !== chat && mid !== account) return true;
      const current = record(payload.curr);
      // 新形式を取り消しと誤認しない。nullだけが取り消しを表す。
      const reaction = {
        userMid: mid,
        kind: payload.curr == null ? -1 : kind(current.predefinedReactionType),
        at: millis(op.createdTime) || now(),
      };
      const rev = /^[0-9]+$/.test(String(op.revision ?? ""))
        ? BigInt(String(op.revision))
        : 0n;
      apply(chat, id, reaction, rev);
    } catch {
      console.error(
        "[reactions] リアクション通知の形式を確認できませんでした。",
      );
    }
    return true;
  }
  function change(chat: string, id: string, desired: unknown): Promise<Json> {
    const active = sync();
    const owner = account;
    const key = `${owner}:${chat}:${id}`;
    if (pending.has(key)) return Promise.reject(new Error("reaction_busy"));
    const task = (async () => {
      const selected = desired === null ? null : kind(desired);
      if (!active || !chatId(chat) || !replyMessageId(id) || selected === 0) {
        throw new Error("invalid_reaction");
      }
      const message = await runtime.findMessage(chat, id);
      if (
        runtime.getClient() !== active ||
        String(active.base.profile?.mid) !== owner
      ) throw new Error("session_changed");
      if (
        runtime.isBlocked(chat) || !message || String(message.id) !== id ||
        !canReact(message, chat, now())
      ) throw new Error("reaction_unavailable");
      const seq = await active.base.getReqseq();
      if (
        runtime.getClient() !== active ||
        String(active.base.profile?.mid) !== owner
      ) throw new Error("session_changed");
      const latest = await runtime.findMessage(chat, id);
      if (
        runtime.getClient() !== active ||
        String(active.base.profile?.mid) !== owner
      ) throw new Error("session_changed");
      if (
        !latest || String(latest.id) !== id || !canReact(latest, chat, now()) ||
        runtime.isBlocked(chat)
      ) {
        throw new Error("reaction_unavailable");
      }
      const before = patches.get(`${chat}:${id}`)?.get(owner);
      if (selected === null) {
        await active.base.talk.cancelReaction({
          cancelReactionRequest: { reqSeq: seq, messageId: BigInt(id) },
        });
      } else {
        await active.base.talk.react({
          id: BigInt(id),
          reaction: selected as 2 | 3 | 4 | 5 | 6 | 7,
          reqSeq: seq,
        });
      }
      if (
        runtime.getClient() === active &&
        String(active.base.profile?.mid) === owner &&
        patches.get(`${chat}:${id}`)?.get(owner) === before
      ) {
        apply(chat, id, {
          userMid: owner,
          kind: selected ?? -1,
          at: Math.max(now(), before?.reaction.at ?? 0),
        });
      }
      return { changed: true };
    })();
    const tracked = task.finally(() => {
      if (pending.get(key) === tracked) pending.delete(key);
    });
    pending.set(key, tracked);
    return tracked;
  }
  function message(chat: string, id: string): Json | undefined {
    sync();
    return known.get(`${chat}:${id}`)?.message;
  }
  return { decorate, handleOperation, change, message };
}
