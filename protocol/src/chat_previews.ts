type Json = Record<string, unknown>;

export type PreviewChat = {
  mid: string;
  preview: string;
  lastActivity: number;
};

type PreviewRuntime = {
  getChats: () => PreviewChat[];
  previewLine: (message: Json) => string;
  fallback: () => string;
  emit: (chat: PreviewChat) => void;
  save: () => Promise<void>;
};

// APIの返却順に依存せず、同時刻なら64ビットのメッセージIDで比較する。
function newestMessage(messages: Json[]): Json | undefined {
  return messages.reduce<Json | undefined>((newest, message) => {
    if (!newest) return message;
    const difference = Number(message.createdTime ?? 0) -
      Number(newest.createdTime ?? 0);
    if (difference !== 0) return difference > 0 ? message : newest;
    const id = String(message.id ?? "");
    const previous = String(newest.id ?? "");
    return /^\d+$/.test(id) && /^\d+$/.test(previous) &&
        BigInt(id) > BigInt(previous)
      ? message
      : newest;
  }, undefined);
}

export function createChatPreviews(runtime: PreviewRuntime) {
  async function updateFromMessages(chatMid: string, messages: Json[]) {
    const chat = runtime.getChats().find((row) => row.mid === chatMid);
    if (!chat) return;
    const newest = newestMessage(messages);
    // 古い履歴やキャッシュで、受信済みの新しいメッセージを上書きしない。
    if (newest && Number(newest.createdTime ?? 0) < chat.lastActivity) return;
    if (!newest && chat.preview) return;
    const preview = newest ? runtime.previewLine(newest) : runtime.fallback();
    if (chat.preview === preview) return;
    chat.preview = preview;
    chat.lastActivity = Math.max(
      chat.lastActivity,
      Number(newest?.createdTime ?? 0),
    );
    runtime.emit({
      mid: chat.mid,
      preview: chat.preview,
      lastActivity: chat.lastActivity,
    });
    await runtime.save();
  }

  return { updateFromMessages };
}
