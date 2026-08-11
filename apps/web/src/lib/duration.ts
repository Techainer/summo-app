/**
 * Lengths of time, in the reader's own language.
 *
 * There were three copies of this — one in `library.ts`, one in `people.ts`, one in `report.ts` —
 * and all three said `phút` and `giờ` whatever language the interface was in. They also disagreed:
 * one rounded a 20-second meeting up to "1 phút", one said "0 phút", one said "20 giây".
 *
 * The words come from `Intl` rather than from the translation catalogue on purpose. A catalogue
 * entry would need a plural rule per language and would be one more thing to get wrong in the
 * eleventh; `Intl.NumberFormat` with a unit already knows that English says "1 minute" and "2
 * minutes" while Vietnamese says "1 phút" and "2 phút", and it knows it for every locale a browser
 * ships. What is left here is the judgement a formatter cannot make: which unit to reach for.
 */

/** Cached because constructing a formatter is the expensive part and a list renders hundreds. */
const formatters = new Map<string, Intl.NumberFormat>();

type Length = "long" | "short";

function unit(
  locale: string,
  name: "second" | "minute" | "hour",
  display: Length,
): Intl.NumberFormat {
  const key = `${locale}:${name}:${display}`;
  const found = formatters.get(key);
  if (found) return found;
  const made = new Intl.NumberFormat(locale, {
    style: "unit",
    unit: name,
    unitDisplay: display,
    maximumFractionDigits: 0,
  });
  formatters.set(key, made);
  return made;
}

/**
 * How long something took, as a person would say it.
 *
 * `2538` reads as "42 minutes", not as a number of seconds nobody converts in their head. Under a
 * minute is reported in seconds rather than rounded to "1 minute", because the difference between a
 * twenty-second note and a one-minute one is the whole of what the reader wanted to know.
 *
 * `seconds <= 0` is an em dash rather than "0 minutes": a meeting with no duration is one that was
 * typed rather than recorded, and a zero invites the reader to wonder what went wrong.
 */
export function formatDuration(seconds: number, locale: string, display: Length = "long"): string {
  if (seconds <= 0) return "—";
  if (seconds < 60) return unit(locale, "second", display).format(Math.round(seconds));

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return unit(locale, "minute", display).format(minutes);

  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  const whole = unit(locale, "hour", display).format(hours);
  return rest === 0 ? whole : `${whole} ${unit(locale, "minute", display).format(rest)}`;
}
