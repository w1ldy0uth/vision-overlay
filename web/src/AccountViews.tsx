import { ItemIcon } from "./InventoryViews";
import {
  adventureExperience,
  completedAchievements,
  showcaseSlots,
} from "./player-profile.mjs";
import type { Profile, ProfileData } from "./api";
import { freshness as freshnessNames, regions } from "./model.mjs";

function Coverage({
  data,
  collection,
}: {
  data: ProfileData;
  collection: string;
}) {
  const domain = data.domains.find((d) => d.collection === collection);
  return (
    <p className="footnote">
      {domain
        ? `${(freshnessNames as Record<string, string>)[domain.freshness] ?? domain.freshness} · ${domain.observed_unix_ms ? new Date(Number(domain.observed_unix_ms)).toLocaleString("ru-RU") : "Время неизвестно"}`
        : "Данные ещё не получены. Нужен захват входа в игру."}
    </p>
  );
}

export function PlayerProfile({
  data,
  profile,
  avatarName,
  catalog,
  openCharacter,
}: {
  data: ProfileData;
  profile?: Profile;
  avatarName: (id: number) => string;
  catalog: any;
  openCharacter: (guid: string) => void;
}) {
  const p = data.collections.player_profile?.find(
    (r) => r.entity_key === "player",
  )?.values;
  const adventure = data.collections.adventure?.[0]?.values;
  const birthday = p?.birthday;
  const picture = catalog.playerProfile?.pictures?.[p?.profile_picture?.id];
  const frame = catalog.playerProfile?.frames?.[p?.profile_picture?.frame_id];
  const portrait =
    picture?.icon ??
    (picture?.avatarId ? catalog.avatarIcons?.[picture.avatarId] : undefined);
  const slots = showcaseSlots(
    p?.show_avatars,
    data.collections.characters ?? [],
  );
  const fields: [string, unknown][] = [
    ["UID", profile?.uid],
    [
      "Сервер",
      profile
        ? ((regions as Record<string, string>)[profile.server_region] ??
          profile.server_region)
        : null,
    ],
    ["Ранг приключений", adventure?.rank],
    ["Опыт приключений", adventureExperience(adventure, catalog.playerProfile)],
    ["Уровень мира", p?.world_level],
    [
      "День рождения",
      birthday?.month && birthday?.day
        ? `${birthday.day}.${String(birthday.month).padStart(2, "0")}`
        : null,
    ],
    [
      "Выполнено достижений",
      completedAchievements(data, p, catalog.achievements),
    ],
  ];
  return (
    <section className="player-profile">
      <section className="account-panel profile-hero">
        <div className="profile-portrait">
          <ItemIcon
            icon={portrait}
            fallback={(p?.nickname ?? "?").slice(0, 1)}
            large
          />
          {frame?.icon && (
            <div className="profile-portrait-frame">
              <ItemIcon icon={frame.icon} fallback="" large />
            </div>
          )}
        </div>
        <div className="profile-identity">
          <h2>{p?.nickname ?? "Профиль игрока"}</h2>
          <p>
            {p?.signature === undefined
              ? "Подпись ещё не получена"
              : p.signature || "Подпись не задана"}
          </p>
        </div>
      </section>
      <section className="account-panel" aria-label="Сведения об аккаунте">
        <dl className="profile-fields">
          {fields.map(([label, value]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd>
                {value === undefined || value === null
                  ? "Не получено"
                  : String(value) || "Не задано"}
              </dd>
            </div>
          ))}
        </dl>
        <Coverage data={data} collection="player_profile" />
      </section>
      <section className="account-panel profile-showcase">
        <h2>Витрина персонажей</h2>
        {!p?.show_avatars && (
          <p className="footnote">Данные витрины ещё не получены.</p>
        )}
        {p?.show_avatars?.length === 0 && <p>Витрина пуста.</p>}
        <div className="showcase-grid">
          {slots.map((slot, index) =>
            slot ? (
              <button
                key={index}
                className="showcase-character"
                disabled={!slot.character}
                onClick={() =>
                  slot.character && openCharacter(slot.character.entity_key)
                }
                aria-label={`Открыть персонажа ${avatarName(slot.displayed.avatar_id)}`}
              >
                <ItemIcon
                  icon={catalog.avatarIcons?.[String(slot.displayed.avatar_id)]}
                  fallback={avatarName(slot.displayed.avatar_id).slice(0, 1)}
                  large
                />
                <span>{avatarName(slot.displayed.avatar_id)}</span>
                <small>Ур. {slot.displayed.level ?? "—"}</small>
              </button>
            ) : (
              <div key={index} className="showcase-empty" aria-hidden="true">
                <span>◇</span>
              </div>
            ),
          )}
        </div>
      </section>
    </section>
  );
}

export function Teams({
  data,
  avatarName,
  avatarIcons,
  openCharacter,
}: {
  data: ProfileData;
  avatarName: (id: number) => string;
  avatarIcons: Record<string, string>;
  openCharacter: (guid: string) => void;
}) {
  const teams = [...(data.collections.teams ?? [])].sort(
    (a, b) => Number(a.entity_key) - Number(b.entity_key),
  );
  const characters = new Map(
    (data.collections.characters ?? []).map((r) => [r.entity_key, r]),
  );
  return (
    <section>
      <Coverage data={data} collection="teams" />
      {!!teams.length && !teams.some((t) => t.values.is_active === true) && (
        <p className="footnote">
          Активный отряд пока не подтверждён. Он определится после переключения
          отряда в игре.
        </p>
      )}
      <div className="team-grid">
        {teams.map((row) => (
          <article className="team-card" key={row.entity_key}>
            <h2>
              <span className="eyebrow">{row.entity_key} · </span>
              {row.values.name || "Без названия"}{" "}
              {row.values.is_active === true && (
                <span className="pill live">Активный</span>
              )}
            </h2>
            {row.confirmation !== "observed" && (
              <p className="footnote">
                Сохранённая запись, не подтверждена текущим снимком
              </p>
            )}
            {row.values.members.length ? (
              <ol className="team-members">
                {row.values.members.map((member: any) => {
                  const character = characters.get(member.guid);
                  return (
                    <li key={member.guid}>
                      {character ? (
                        <button
                          className="team-member"
                          aria-label={`Открыть персонажа ${avatarName(character.values.avatar_id)}`}
                          onClick={() => openCharacter(member.guid)}
                        >
                          <ItemIcon
                            icon={avatarIcons[character.values.avatar_id]}
                            fallback={avatarName(
                              character.values.avatar_id,
                            ).slice(0, 1)}
                            large
                          />
                          <span>{avatarName(character.values.avatar_id)}</span>
                        </button>
                      ) : (
                        <span
                          className="team-member team-member-unknown"
                          title={`GUID: ${member.guid}`}
                        >
                          <ItemIcon fallback="?" large />
                          <span>Неизвестный персонаж</span>
                        </span>
                      )}
                    </li>
                  );
                })}
              </ol>
            ) : (
              <p className="footnote">Пустой отряд</p>
            )}
          </article>
        ))}
      </div>
    </section>
  );
}
