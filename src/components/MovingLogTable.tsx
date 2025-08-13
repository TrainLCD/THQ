import { memo } from "react";
import type { LocationData } from "~/domain/commands";
import { STATE_ICONS } from "~/domain/emoji";
import { BAD_ACCURACY_THRESHOLD } from "~/domain/threshold";
import { toKMH } from "~/utils/unit";

export const MovingLogTable = memo(
  ({ movingLogs }: { movingLogs: LocationData[] }) => {
    return (
      <div className="overflow-y-auto max-h-96 overscroll-none border border-gray-200 dark:border-white/15 rounded-md">
        <table className="bg-white shadow dark:bg-white/5 w-full border-collapse">
          <caption className="sr-only">移動ログ一覧</caption>
          <thead className="sticky top-0 z-10 bg-white dark:bg-black border-b border-b-gray-200 dark:border-b-white/15">
            <tr>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15 w-16"
              >
                state
              </th>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15"
              >
                timestamp
              </th>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15"
              >
                coordinates
              </th>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15"
              >
                speed
              </th>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15"
              >
                accuracy
              </th>
              <th
                scope="col"
                className="p-2 border border-gray-200 dark:border-white/15"
              >
                device
              </th>
            </tr>
          </thead>
          <tbody>
            {movingLogs.map((t) => (
              <tr key={t.id}>
                <td
                  className="p-2 border border-gray-200 dark:border-white/15 w-16 text-center"
                  aria-label={t.state}
                  title={t.state}
                >
                  {STATE_ICONS[t.state] ?? "?"}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                  {new Date(t.timestamp).toLocaleString("ja-JP", {
                    year: "numeric",
                    month: "2-digit",
                    day: "2-digit",
                    hour: "2-digit",
                    minute: "2-digit",
                    second: "2-digit",
                    hour12: false,
                  })}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                  {t.lat == null || t.lon == null
                    ? "—"
                    : `${t.lat.toFixed(5)}, ${t.lon.toFixed(5)}`}
                </td>
                {t.speed == null ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    —
                  </td>
                ) : t.speed < 0 ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold tabular-nums whitespace-nowrap">
                    {t.speed.toFixed(2)} m/s ({toKMH(t.speed).toFixed(2)} km/h)
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    {t.speed.toFixed(2)} m/s ({toKMH(t.speed).toFixed(2)} km/h)
                  </td>
                )}
                {t.accuracy == null ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    —
                  </td>
                ) : t.accuracy > BAD_ACCURACY_THRESHOLD ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold tabular-nums whitespace-nowrap">
                    {t.accuracy.toFixed(2)}m
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    {t.accuracy.toFixed(2)}m
                  </td>
                )}
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono whitespace-nowrap">
                  {t.device}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
);
