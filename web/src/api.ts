export type Row = {
  source_collection?: string;
  entity_key: string;
  values: Record<string, any>;
  confirmation: string;
  value_format: string;
  revision: string;
  guids?: string[];
};
export type Domain = {
  collection: string;
  freshness: string;
  scope: string;
  reason: string;
  observed_unix_ms: string;
};
export type Profile = {
  profile_id: string;
  uid: string;
  server_region: string;
  domains: Domain[];
};
export type State = {
  revision: string;
  active_game_profile_id: string | null;
  active_game_session?: string | null;
  profiles: Profile[];
  mode: string;
};
export type ProfileData = {
  revision: string;
  profile_id: string;
  mode: string;
  domains: Domain[];
  collections: Record<string, Row[]>;
  material_totals: { item_id: number; count: string }[];
};
export async function get<T = any>(
  url: string,
  signal?: AbortSignal,
): Promise<T> {
  const r = await fetch(url, { signal });
  if (!r.ok) {
    let body;
    try {
      body = await r.json();
    } catch {}
    throw new Error(body?.error?.message ?? `HTTP ${r.status}`);
  }
  return r.json();
}
export async function pages(url: string, signal?: AbortSignal) {
  let cursor: string | null = null;
  const items: any[] = [];
  do {
    const data: { items: any[]; next_cursor: string | null } = await get(
      `${url}${url.includes("?") ? "&" : "?"}limit=1000${cursor ? "&cursor=" + encodeURIComponent(cursor) : ""}`,
      signal,
    );
    items.push(...data.items);
    cursor = data.next_cursor;
  } while (cursor);
  return items;
}
