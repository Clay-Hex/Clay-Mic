function pad(value: number, length = 2): string {
  return value.toString().padStart(length, "0");
}

export interface FormattedTime {
  /** Relative label shown inline (今天 / 昨天 / 前天 / 更早的日期). */
  text: string;
  /** Exact local time shown on hover. */
  title: string;
}

/** Format an ISO timestamp into a relative label plus an exact-time tooltip. */
export function formatTimestamp(iso: string): FormattedTime {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return { text: iso, title: iso };
  }

  const now = new Date();
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  const title = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )} ${time}.${pad(date.getMilliseconds(), 3)}`;

  const startOfDay = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);

  let text = time;
  if (days === 1) {
    text = `昨天${time}`;
  } else if (days === 2) {
    text = `前天${time}`;
  } else if (days > 2) {
    text = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
      date.getDate(),
    )} ${time}`;
  }

  return { text, title };
}
