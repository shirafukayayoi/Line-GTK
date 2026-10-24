import type { CommandRuntime } from "./commands.ts";

type Json = Record<string, unknown>;
export type HistoryCursor = { messageId: string; deliveredTime: string };

function integer(value: unknown, positive = false): string | null {
  if (typeof value === "number" && !Number.isSafeInteger(value)) return null;
  const text = String(value ?? "");
  if (!/^\d+$/.test(text)) return null;
  const n = BigInt(text);
  return n >= (positive ? 1n : 0n) && n <= 9223372036854775807n
    ? n.toString()
    : null;
}

export function cursorOf(value: unknown): HistoryCursor | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Json;
  const messageId = integer(raw.messageId ?? raw.id, true);
  const deliveredTime = integer(raw.deliveredTime);
  return messageId && deliveredTime ? { messageId, deliveredTime } : null;
}

function compare(a: HistoryCursor, b: HistoryCursor): number {
  const time = BigInt(a.deliveredTime) - BigInt(b.deliveredTime);
  const diff = time || BigInt(a.messageId) - BigInt(b.messageId);
  return diff < 0n ? -1 : diff > 0n ? 1 : 0;
}

export function oldestCursor(messages: unknown[]): HistoryCursor | null {
  let oldest: HistoryCursor | null = null;
  for (const message of messages) {
    const raw = message as Json;
    const cursor = cursorOf(raw?.historyCursor ?? raw);
    if (cursor && (!oldest || compare(cursor, oldest) < 0)) oldest = cursor;
  }
  return oldest;
}

export function withHistoryCursor(summary: Json, raw: unknown): Json {
  return { ...summary, historyCursor: cursorOf(raw) };
}

export function historyState(messages: Json[]) {
  return {
    historyCursor: oldestCursor(messages),
    // 古いキャッシュには配信時刻がないため、追加取得時に取得位置を補完する。
    hasMore: messages.some((message) => integer(message.id, true) !== null),
  };
}

export function createHistoryService(runtime: CommandRuntime) {
  return async function fetchOlder(
    id: number | string | null,
    chatMid: string,
    value: unknown,
    limit = 40,
  ) {
    const client = runtime.getClient();
    if (!client) return runtime.fail(id, "not_logged_in");
    if (!chatMid) return runtime.fail(id, "missing_chat");
    const count = Math.max(1, Math.min(80, Math.trunc(limit) || 40));
    try {
      let before = cursorOf(value);
      if (value != null && !before) throw new Error("invalid_history_cursor");
      if (!before) {
        const boxes = await client.base.talk.getMessageBoxes({
          messageBoxListRequest: {},
        });
        const last = boxes.messageBoxes?.find((box) => box.id === chatMid)
          ?.lastDeliveredMessageId;
        const latest = cursorOf(last);
        if (latest) {
          const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
            request: {
              messageBoxId: chatMid,
              endMessageId: {
                messageId: BigInt(latest.messageId),
                deliveredTime: BigInt(latest.deliveredTime),
              },
              messagesCount: count,
            },
          });
          before = oldestCursor(raw);
        }
      }
      if (runtime.getClient() !== client) throw new Error("session_changed");
      if (!before) {
        runtime.ok(id, { messages: [], historyCursor: null, hasMore: false });
        return;
      }
      const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
        request: {
          messageBoxId: chatMid,
          endMessageId: {
            messageId: BigInt(before.messageId),
            deliveredTime: BigInt(before.deliveredTime),
          },
          messagesCount: count,
        },
      });
      // 境界を再送するサーバーでも重複せず、取得位置が進まなければ停止する。
      const older = raw.filter((message) => {
        const cursor = cursorOf(message);
        return cursor && compare(cursor, before!) < 0;
      });
      const next = oldestCursor(older);
      const messages: Json[] = [];
      for (const message of older) {
        messages.push(
          withHistoryCursor(
            await runtime.summarizeRawMessage(message, { withMedia: false }),
            message,
          ),
        );
      }
      messages.sort((a, b) => Number(a.createdTime) - Number(b.createdTime));
      const prepared = await runtime.forUiMessages(messages, chatMid);
      const ids = new Set(messages.map((message) => String(message.id)));
      const times = messages.map((message) => Number(message.createdTime));
      const ui = prepared.filter((message) =>
        ids.has(String(message.id)) ||
        (message.chatEvent &&
          Number(message.createdTime) >= Math.min(...times) &&
          Number(message.createdTime) <= Math.max(...times))
      );
      const cached = runtime.msgCache.get(chatMid);
      const previous = cached?.messages ??
        await runtime.loadDiskMessages(chatMid) ?? [];
      if (runtime.getClient() !== client) throw new Error("session_changed");
      const merged = new Map(
        previous.map((message) => [String(message.id), message]),
      );
      for (const message of messages) merged.set(String(message.id), message);
      const all = [...merged.values()].sort((a, b) =>
        Number(a.createdTime) - Number(b.createdTime)
      );
      runtime.msgCache.set(chatMid, {
        at: cached?.at ?? 0,
        messages: all,
        latestCount: cached?.latestCount ?? previous.length,
      });
      await runtime.saveDiskMessages(chatMid, all);
      if (runtime.getClient() !== client) throw new Error("session_changed");
      runtime.ok(id, {
        messages: ui,
        historyCursor: next ?? before,
        hasMore: next !== null,
      });
      // 履歴の画像取得で、最新メッセージの画像取得を中断しない。
      void runtime.hydrateMedia(messages, chatMid, { isolated: true });
    } catch (error) {
      runtime.fail(id, error instanceof Error ? error.message : String(error));
    }
  };
}
