import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("English preserves interpolation, spacing, and verified game names", async () => {
  const storage = globalThis.localStorage;
  const fetch = globalThis.fetch;
  globalThis.localStorage = { getItem: () => "en" };
  globalThis.fetch = async () => ({
    ok: true,
    json: async () => JSON.parse(await readFile(new URL("../public/game-text-en.json", import.meta.url), "utf8")),
  });
  try {
    const i18n = await import("../src/i18n.mjs");
    assert.equal(i18n.uiLocale, "en-US");
    assert.equal(i18n.tr(" · нужно ещё {0} ", 7), " · 7 more needed ");
    const gameName = { item: "Семена одуванчика", unlocalized: "Player-written signature", count: 4 };
    await i18n.loadGameLanguage();
    assert.deepEqual(i18n.localizeGameData(gameName), {
      item: "Dandelion Seed", unlocalized: "Player-written signature", count: 4,
    });
    assert.equal(gameName.item, "Семена одуванчика");
  } finally {
    if (storage === undefined) delete globalThis.localStorage;
    else globalThis.localStorage = storage;
    globalThis.fetch = fetch;
  }
});
