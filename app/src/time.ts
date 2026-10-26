const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
const dateFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const dateTimeFmt = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** "just now", "5 minutes ago", "yesterday", then a date. */
export function relativeTime(ms: number): string {
  const diff = (ms - Date.now()) / 1000;
  const abs = Math.abs(diff);
  if (abs < 45) return "just now";
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 7 * 86400) return rtf.format(Math.round(diff / 86400), "day");
  return dateFmt.format(ms);
}

export function fullDate(ms: number): string {
  return dateTimeFmt.format(ms);
}
