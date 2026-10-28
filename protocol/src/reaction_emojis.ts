import type { Client } from "@evex/linejs";

export type PaidReaction = {
  productId: string;
  emojiId: string;
  resourceType: number;
  version: number;
};
type Json = Record<string, unknown>;
export const object = (value: unknown): Json =>
  value && typeof value === "object" ? value as Json : {};
export function paidReaction(value: unknown): PaidReaction | null {
  const raw = object(value);
  const productId = String(raw.productId ?? raw["1"] ?? "").toLowerCase();
  const emojiId = String(raw.emojiId ?? raw["2"] ?? "");
  const resourceType = Number(raw.resourceType ?? raw["3"] ?? 1);
  const version = Number(raw.version ?? raw["4"] ?? 0);
  return /^[a-f0-9]{24}$/.test(productId) && /^[0-9]{1,5}$/.test(emojiId) &&
      [1, 2].includes(resourceType) && Number.isSafeInteger(version) &&
      version >= 0
    ? { productId, emojiId, resourceType, version }
    : null;
}
export const emojiKey = (emoji: PaidReaction) =>
  `${emoji.productId}_${emoji.emojiId}`;

// AndroidのReactionType unionを補う。依存ライブラリや共有キャッシュは変更しない。
export function sendEmojiReaction(
  client: Client,
  seq: number,
  id: string,
  emoji: PaidReaction,
): Promise<unknown> {
  return client.base.request.request(
    [[12, 1, [
      [8, 1, seq],
      [10, 2, BigInt(id)],
      [12, 3, [[12, 2, [
        [11, 1, emoji.productId],
        [11, 2, emoji.emojiId],
        [8, 3, emoji.resourceType],
        [10, 4, BigInt(emoji.version)],
      ]]]],
    ]]],
    "react",
    client.base.talk.protocolType,
    true,
    client.base.talk.requestPath,
  );
}
