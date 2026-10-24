import { replyMessageId } from "./replies.ts";
import type { Client } from "@evex/linejs";

class ScheduledGuardError extends Error {}

export type ScheduledText = {
  accountMid: string;
  reservationId: string;
  chatMid: string;
  text: string;
  sendAt: number;
  replyToMessageId?: string;
};

// 通常送信のフォールバック・再試行から分離し、予約1件につき一度だけ開始する。
export function createScheduledSender<T>(runtime: {
  account: () => string;
  ready: () => boolean;
  blocked: (mid: string) => boolean;
  session?: () => unknown;
  send: (input: ScheduledText, check: () => void) => Promise<T>;
  now?: () => number;
}) {
  const attempted = new Set<string>();
  return async (params: Record<string, unknown>): Promise<T> => {
    const {
      accountMid,
      reservationId,
      chatMid,
      text,
      sendAt,
      replyToMessageId,
    } = params;
    if (
      typeof accountMid !== "string" || !/^u\S{1,127}$/.test(accountMid) ||
      typeof reservationId !== "string" ||
      !/^[1-9][0-9]{0,19}$/.test(reservationId) ||
      typeof chatMid !== "string" || !/^[ucr]\S{1,127}$/.test(chatMid) ||
      typeof text !== "string" || !text.trim() ||
      Array.from(text).length > 10_000 ||
      typeof sendAt !== "number" || !Number.isSafeInteger(sendAt) || sendAt <= 0
    ) throw new Error("scheduled_invalid");
    const related = replyToMessageId == null
      ? undefined
      : replyMessageId(replyToMessageId);
    if (related === null) throw new Error("scheduled_invalid");
    const owner = runtime.session?.();
    const check = () => {
      const now = (runtime.now ?? Date.now)();
      if (now < sendAt || now > sendAt + 2000) {
        throw new ScheduledGuardError("scheduled_expired");
      }
      if (
        runtime.account() !== accountMid || !runtime.ready() ||
        runtime.session?.() !== owner
      ) throw new ScheduledGuardError("scheduled_unavailable");
      if (runtime.blocked(chatMid)) {
        throw new ScheduledGuardError("contact_blocked_send");
      }
    };
    check();
    const key = `${accountMid}:${reservationId}`;
    if (attempted.has(key)) throw new Error("scheduled_duplicate");
    if (attempted.size >= 10_000) throw new Error("scheduled_limit");
    attempted.add(key);
    try {
      return await runtime.send({
        accountMid,
        reservationId,
        chatMid,
        text,
        sendAt,
        replyToMessageId: related,
      }, check);
    } catch (error) {
      if (error instanceof ScheduledGuardError) throw error;
      // 通信失敗でもサーバー受理済みの可能性がある。別経路へ再送しない。
      throw new Error("scheduled_send_unknown");
    }
  };
}

// 暗号鍵取得・連番保存・認証更新などのawait後にも、実際の通信直前で確認する。
// クライアント本体を書き換えず、予約1件だけに適用する通信オブジェクトを作る。
export function guardedScheduledTalk(
  base: Client["base"],
  sendAt: number,
  check: () => void,
) {
  const scoped = new Proxy(base, {
    get(target, key) {
      if (key === "request") return request;
      if (key === "getReqseq") {
        return async (...args: unknown[]) => {
          const seq = await Reflect.apply(target.getReqseq, target, args);
          check();
          return seq;
        };
      }
      if (key === "fetch") {
        return (info: RequestInfo | URL, init?: RequestInit) => {
          check();
          const original = new Request(info, init);
          const remaining = Math.max(1, sendAt + 2000 - Date.now());
          return target.fetch(
            new Request(original, {
              signal: AbortSignal.any([
                original.signal,
                AbortSignal.timeout(remaining),
              ]),
            }),
          );
        };
      }
      const value = Reflect.get(target, key, target);
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
  const RequestClient = base.request.constructor as new (
    client: Client["base"],
  ) => typeof base.request;
  const request = new RequestClient(scoped);
  const talk = Object.create(base.talk) as typeof base.talk;
  talk.client = scoped;
  return talk;
}
