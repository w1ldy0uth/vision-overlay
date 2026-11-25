export const serverOffsets = {
  os_euro: '+01:00',
  os_asia: '+08:00',
  os_usa: '-05:00',
  os_cht: '+08:00',
};

const dayMs = 86_400_000;

export function serverDay(region, now) {
  const offset = serverOffsets[region];
  const reset = Date.parse(`1970-01-01T04:00:00${offset}`);
  const elapsedDays = Math.floor((now - reset) / dayMs);
  return {
    weekday: new Date(elapsedDays * dayMs).getUTCDay(),
    nextReset: reset + (elapsedDays + 1) * dayMs,
  };
}

export function nextServerWeekReset(region, now) {
  const { weekday, nextReset } = serverDay(region, now);
  return nextReset + ((7 - weekday) % 7) * dayMs;
}
