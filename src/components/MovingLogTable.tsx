import { memo } from "react";
import type { LocationData } from "~/domain/commands";
import { STATE_ICONS } from "~/domain/emoji";
import { BAD_ACCURACY_THRESHOLD } from "~/domain/threshold";
import { toKMH } from "~/utils/unit";

export const MovingLogTable = memo(
  ({ movingLogs }: { movingLogs: LocationData[] }) => {
    return (
      <div className="overflow-y-scroll max-h-96 overscroll-none border border-gray-200 dark:border-white/15 rounded-md">
        <table className="bg-white shadow dark:bg-white/5 w-full">
          <thead className="sticky top-0 bg-white dark:bg-black border-b-1 borer-b-gray-200 dark:border-b-white/15">
            <tr>
              <th className="p-2 border border-gray-200 dark:border-white/15 w-16">
                state
              </th>
              <th className="p-2 border border-gray-200 dark:border-white/15">
                timestamp
              </th>
              <th className="p-2 border border-gray-200 dark:border-white/15">
                coordinates
              </th>
              <th className="p-2 border border-gray-200 dark:border-white/15">
                speed
              </th>
              <th className="p-2 border border-gray-200 dark:border-white/15">
                accuracy
              </th>
              <th className="p-2 border border-gray-200 dark:border-white/15">
                device
              </th>
            </tr>
          </thead>
          <tbody>
            {movingLogs.map((t) => (
              <tr key={t.id}>
                <td className="p-2 border border-gray-200 dark:border-white/15 w-16 text-center">
                  {STATE_ICONS[t.state]}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15">
                  {new Date(t.timestamp).toLocaleString()}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15">
                  {t.lon == null || t.lat == null
                    ? "—"
                    : `${t.lon.toFixed(5)}, ${t.lat.toFixed(5)}`}
                </td>
                {t.speed == null ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15">—</td>
                ) : t.speed < 0 ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold">
                    {t.speed.toFixed(2)}m/s ({toKMH(t.speed).toFixed(2)}km/h)
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15">
                    {t.speed.toFixed(2)}m/s ({toKMH(t.speed).toFixed(2)}km/h)
                  </td>
                )}
                {(t?.accuracy ?? 0) > BAD_ACCURACY_THRESHOLD ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold">
                    {t.accuracy?.toFixed(2)}m
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15">
                    {t.accuracy?.toFixed(2)}m
                  </td>
                )}
                <td className="p-2 border border-gray-200 dark:border-white/15">
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
