type Json = Record<string, unknown>;

export type CallHistory = {
  group?: boolean;
  video: boolean;
  status: "completed" | "unanswered" | "unknown";
  durationMs: number | null;
};

export type CallHistoryLabels = {
  groupCall: string;
  groupVideo: string;
  callIncoming: string;
  callOutgoing: string;
  callMissed: string;
  callUnanswered: string;
  videoIncoming: string;
  videoOutgoing: string;
  videoMissed: string;
  videoUnanswered: string;
};

const legacyHistory = /^Call History\s*:\s*(\d+) millisecs, Result:\s*\d+$/i;

function duration(value: unknown): number | null {
  if (
    value == null || value === "" ||
    (typeof value !== "number" && typeof value !== "string")
  ) return null;
  if (typeof value === "string" && !/^\d+$/.test(value)) return null;
  const number = Number(value);
  return Number.isSafeInteger(number) && number >= 0 ? number : null;
}

export function callHistoryOf(message: Json): CallHistory | null {
  const metadata = (message.contentMetadata ?? {}) as Json;
  if (
    String(message.contentType).toUpperCase() !== "CALL" &&
    message.contentType !== 6 && metadata.ORGCONTP !== "CALL"
  ) return null;
  const saved = (message.callHistory ?? {}) as Json;
  const legacy = legacyHistory.exec(String(message.text ?? "").trim());
  const durationMs = duration(metadata.DURATION) ??
    duration(saved.durationMs) ??
    duration(message.durationMs) ?? duration(legacy?.[1]);
  const result = String(metadata.RESULT ?? "").toUpperCase();
  // 数値の結果コードや通話時間ゼロだけでは、不在着信と断定しない。
  const unanswered = saved.status === "unanswered" ||
    ["MISSED", "CANCELED", "CANCELLED", "NO_ANSWER"].includes(result);
  const group = saved.group === true ||
    [message.to, message.from, message.chatMid]
      .some((mid) => typeof mid === "string" && /^[cr]/.test(mid));
  return {
    ...(group ? { group: true } : {}),
    video: metadata.TYPE === "V" || metadata.GC_MEDIA_TYPE === "VIDEO" ||
      saved.video === true,
    status: durationMs != null && durationMs > 0
      ? "completed"
      : unanswered
      ? "unanswered"
      : "unknown",
    durationMs,
  };
}

export function formatCallDuration(milliseconds: number): string {
  const seconds = Math.floor(milliseconds / 1000);
  const minutes = Math.floor(seconds / 60);
  const remainder = String(seconds % 60).padStart(2, "0");
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}:${
      String(minutes % 60).padStart(2, "0")
    }:${remainder}`
    : `${minutes}:${remainder}`;
}

export function callHistoryPreview(
  message: Json,
  labels: CallHistoryLabels,
): string | null {
  const history = callHistoryOf(message);
  if (!history) return null;
  const missed = history.status === "unanswered";
  const title = history.group
    ? history.video ? labels.groupVideo : labels.groupCall
    : history.video
    ? message.mine
      ? missed ? labels.videoUnanswered : labels.videoOutgoing
      : missed
      ? labels.videoMissed
      : labels.videoIncoming
    : message.mine
    ? missed ? labels.callUnanswered : labels.callOutgoing
    : missed
    ? labels.callMissed
    : labels.callIncoming;
  return history.durationMs != null && history.durationMs > 0
    ? `${title} · ${formatCallDuration(history.durationMs)}`
    : title;
}

// 旧版がそのまま保存した内部表現は、再取得して表示用のプレビューに直す。
export function isLegacyCallPreview(preview: string): boolean {
  const body = preview.replace(/^(?:You|They|คุณ|เขา|自分|相手): /, "");
  return legacyHistory.test(body.trim());
}

// グループに残った旧版の個別通話プレビューは、本文を再取得して作り直す。
export function isOutdatedGroupCallPreview(
  mid: string,
  preview: string,
): boolean {
  if (!/^[cr]/.test(mid)) return false;
  const body = preview.replace(/^(?:You|They|คุณ|เขา|自分|相手): /, "");
  const title = body.split(" · ")[0];
  return [
    "着信がありました",
    "発信しました",
    "不在着信",
    "応答なし",
    "ビデオ通話の着信",
    "ビデオ通話の発信",
    "ビデオ通話の不在着信",
    "ビデオ通話の応答なし",
    "Incoming call",
    "Outgoing call",
    "Missed call",
    "Unanswered call",
    "Incoming video call",
    "Outgoing video call",
    "Missed video call",
    "Unanswered video call",
    "มีสายเรียกเข้า",
    "โทรออก",
    "สายที่ไม่ได้รับ",
    "ไม่มีผู้รับสาย",
    "สายวิดีโอเรียกเข้า",
    "โทรวิดีโอออก",
    "สายวิดีโอที่ไม่ได้รับ",
    "ไม่มีผู้รับสายวิดีโอ",
  ].includes(title);
}
