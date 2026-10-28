import type { Client } from "@evex/linejs";
import {
  emojiKey,
  object,
  type PaidReaction,
  paidReaction,
} from "./reaction_emojis.ts";

type Package = {
  id: string;
  name: string;
  version: number;
  resourceType: number;
};
type Choice = { emoji: PaidReaction; key: string; label: string };
const ORIGINAL = "670e0cce840a8236ddd4ee4c";
const number = (value: unknown) => {
  const n = Number(value);
  return Number.isSafeInteger(n) && n > 0 ? n : 0;
};
const resource = (value: unknown) =>
  value === "ANIMATION" || Number(value) === 2 ? 2 : 1;

export function createReactionCatalog(runtime: {
  getClient: () => Client | null;
  images: (keys: string[]) => void;
  fetch?: typeof fetch;
}) {
  let owner: Client | null = null;
  let account = "";
  let listing: Promise<{ packages: Package[]; warning: boolean }> | undefined;
  let until = 0;
  const details = new Map<string, Promise<Choice[]>>();
  function sync() {
    const client = runtime.getClient();
    const mid = String(client?.base.profile?.mid ?? "");
    if (owner !== client || account !== mid) {
      owner = client;
      account = mid;
      listing = undefined;
      details.clear();
      until = 0;
    }
    if (!client || !mid) throw new Error("not_logged_in");
    return client;
  }
  async function list(force = false) {
    const client = sync();
    const mid = account;
    if (force) {
      listing = undefined;
      details.clear();
    }
    if (!listing || Date.now() > until) {
      until = Date.now() + 300_000;
      listing = (async () => {
        const packages = new Map<string, Package>([[ORIGINAL, {
          id: ORIGINAL,
          name: "LINE",
          version: 1,
          resourceType: 1,
        }]]);
        let warning = false;
        try {
          for (let offset = 0; offset < 1000;) {
            const response = object(
              await client.base.request.request(
                [
                  [11, 2, "sticonshop"],
                  [8, 3, offset],
                  [8, 4, 100],
                  [12, 5, [[11, 1, "ja"], [11, 2, "JP"]]],
                ],
                "getOwnedProductSummaries",
                4,
                true,
                "/TSHOP4",
              ),
            );
            if (
              runtime.getClient() !== client ||
              String(client.base.profile?.mid) !== mid
            ) {
              throw new Error("session_changed");
            }
            const rows = response.productList ?? response["1"];
            if (!Array.isArray(rows)) throw new Error("emoji_catalog_invalid");
            const before = packages.size;
            for (const input of rows) {
              const row = object(input);
              const id = String(row.id ?? row["1"] ?? "").toLowerCase();
              const version = number(row.latestVersion ?? row["21"]);
              const validUntil = Number(row.validUntil ?? row["94"] ?? 0);
              if (
                !/^[a-f0-9]{24}$/.test(id) || !version ||
                (validUntil > 0 && validUntil < Date.now())
              ) continue;
              const types = object(row.productTypeSummary ?? row["93"]);
              const summary = object(types.sticonSummary ?? types["3"]);
              packages.set(id, {
                id,
                name: String(row.name ?? row["11"] ?? id).slice(0, 100),
                version,
                resourceType: resource(
                  summary.sticonResourceType ?? summary["3"],
                ),
              });
            }
            offset += rows.length;
            if (
              rows.length < 100 ||
              (number(response.totalSize ?? response["3"]) > 0 &&
                offset >= number(response.totalSize ?? response["3"]))
            ) break;
            if (packages.size === before) {
              throw new Error("emoji_catalog_no_progress");
            }
            if (offset >= 1000) warning = true;
          }
        } catch {
          warning = true;
        }
        if (
          runtime.getClient() !== client ||
          String(client.base.profile?.mid) !== mid
        ) throw new Error("session_changed");
        return { packages: [...packages.values()], warning };
      })();
    }
    return await listing;
  }
  async function pack(id: string) {
    const client = sync();
    const mid = account;
    const entry = (await list()).packages.find((p) => p.id === id);
    if (!entry) throw new Error("emoji_not_owned");
    let task = details.get(id);
    if (!task) {
      task = (async () => {
        const response = await (runtime.fetch ?? fetch)(
          `https://stickershop.line-scdn.net/sticonshop/v1/${id}/sticon/iphone/meta.json`,
          { signal: AbortSignal.timeout(8000), redirect: "error" },
        );
        if (!response.ok || !response.body) {
          throw new Error("emoji_package_unavailable");
        }
        let text = "";
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let length = 0;
        try {
          while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            length += value.length;
            if (length > 1024 * 1024) {
              throw new Error("emoji_package_too_large");
            }
            text += decoder.decode(value, { stream: true });
          }
          text += decoder.decode();
        } finally {
          await reader.cancel().catch(() => {});
        }
        const meta = object(JSON.parse(text));
        if (meta.productId && meta.productId !== id) {
          throw new Error("emoji_package_mismatch");
        }
        if (!Array.isArray(meta.orders)) {
          throw new Error("emoji_package_invalid");
        }
        const labels = object(meta.altTexts);
        const seen = new Set<string>();
        const choices: Choice[] = [];
        for (const value of meta.orders.slice(0, 500)) {
          const emoji = paidReaction({
            productId: id,
            emojiId: value,
            resourceType: entry.resourceType,
            version: entry.version,
          });
          if (!emoji || seen.has(emoji.emojiId)) continue;
          seen.add(emoji.emojiId);
          choices.push({
            emoji,
            key: emojiKey(emoji),
            label: String(labels[emoji.emojiId] ?? `LINE ${emoji.emojiId}`)
              .slice(0, 100),
          });
        }
        if (
          runtime.getClient() !== client ||
          String(client.base.profile?.mid) !== mid
        ) throw new Error("session_changed");
        if (!choices.length) throw new Error("emoji_package_empty");
        runtime.images(choices.map((c) => c.key));
        return choices;
      })();
      details.set(id, task);
      void task.catch(() => {
        if (details.get(id) === task) details.delete(id);
      });
    }
    return await task;
  }
  async function resolve(emoji: PaidReaction): Promise<PaidReaction> {
    const client = sync();
    const mid = account;
    const choice = (await pack(emoji.productId)).find((c) =>
      c.emoji.emojiId === emoji.emojiId
    );
    if (
      runtime.getClient() !== client || String(client.base.profile?.mid) !== mid
    ) throw new Error("session_changed");
    if (!choice) throw new Error("emoji_not_available");
    return choice.emoji;
  }
  return { list, pack, resolve };
}
