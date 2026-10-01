import dayjs from "dayjs";
import utc from "dayjs/plugin/utc";

dayjs.extend(utc);

/// A server timestamp as local, readable text — or null, never "Invalid Date".
///
/// Every PDS should send ISO 8601 UTC, but one shipped java.sql.Timestamp's
/// "2026-10-01 18:23:20.014" for a while: no T, no zone. Strict Date parsers
/// refuse that outright and lenient ones read it in the viewer's zone, hours
/// off. A stamp without an explicit zone is therefore read as UTC, and anything
/// unparseable renders as absent rather than broken.
export function formatWhen(value: string | null | undefined): string | null {
  if (!value) return null;
  const zoned = /([zZ]|[+-]\d{2}:?\d{2})$/.test(value);
  const parsed = zoned ? dayjs(value) : dayjs.utc(value);
  return parsed.isValid() ? parsed.local().format("MMM D, YYYY, h:mm A") : null;
}
