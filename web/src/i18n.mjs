import english from "./locales/en.json" with { type: "json" };

function savedLanguage() {
  try {
    return globalThis.localStorage?.getItem("vo.language") === "en" ? "en" : "ru";
  } catch {
    return "ru";
  }
}

export const language = savedLanguage();
export const uiLocale = language === "en" ? "en-US" : "ru-RU";

/** @param {"en" | "ru"} next */
export function setLanguage(next) {
  if (next === language) return;
  localStorage.setItem("vo.language", next);
  location.reload();
}

/** @param {string} message @param {...unknown} values */
export function tr(message, ...values) {
  const key = message.trim();
  const translated = language === "en" && Object.hasOwn(english, key)
    ? english[/** @type {keyof typeof english} */ (key)] : key;
  const result = values.length
    ? translated.replace(/\{(\d+)\}/g, (match, index) => index < values.length ? String(values[index]) : match)
    : translated;
  if (!key) return message;
  return message.slice(0, message.length - message.trimStart().length) + result + message.slice(message.trimEnd().length);
}

/** @type {Record<string, string>} */
let gameTexts = {};

export async function loadGameLanguage() {
  if (language !== "en") return;
  const response = await fetch("/game-text-en.json");
  if (!response.ok) throw Error("English game text unavailable");
  gameTexts = await response.json();
}

/** @param {string} text */
export function gameText(text) {
  if (language !== "en" || typeof text !== "string") return text;
  if (Object.hasOwn(gameTexts, text)) return gameTexts[text];
  const achievement = /^Достижение ID (\d+)$/.exec(text);
  return achievement ? `Achievement ID ${achievement[1]}` : tr(text);
}

/** @template T @param {T} data @returns {T} */
export function localizeGameData(data) {
  if (language !== "en" || data === null) return data;
  if (typeof data === "string") return /** @type {T} */ (gameText(data));
  if (Array.isArray(data)) return /** @type {T} */ (data.map(localizeGameData));
  if (data && typeof data === "object")
    return /** @type {T} */ (Object.fromEntries(Object.entries(data).map(([key, value]) => [key, localizeGameData(value)])));
  return data;
}
