import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useRef } from "react";
import type { LocationData } from "~/domain/commands";
import { STATE_ICONS } from "~/domain/emoji";
import { BAD_ACCURACY_THRESHOLD } from "~/domain/threshold";
import { toKMH } from "~/utils/unit";

export const MovingLogTable = memo(
  ({ movingLogs }: { movingLogs: LocationData[] }) => {
    const parentRef = useRef<HTMLDivElement | null>(null);
    const rowVirtualizer = useVirtualizer({
      count: movingLogs.length,
      getScrollElement: () => parentRef.current,
      estimateSize: () => 40,
    });

    return (
      <div
        className="overflow-y-auto max-h-96 overscroll-none border border-gray-200 dark:border-white/15 rounded-md"
        ref={parentRef}
      >
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
            {rowVirtualizer.getVirtualItems().map((vItem) => (
              <tr
                key={vItem.key}
                data-index={vItem.index}
                ref={rowVirtualizer.measureElement}
              >
                <td
                  className="p-2 border border-gray-200 dark:border-white/15 w-16 text-center"
                  aria-label={movingLogs[vItem.index]?.state}
                  title={movingLogs[vItem.index]?.state}
                >
                  {STATE_ICONS[movingLogs[vItem.index]?.state] ?? "?"}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                  {new Date(movingLogs[vItem.index]?.timestamp).toLocaleString(
                    "ja-JP",
                    {
                      year: "numeric",
                      month: "2-digit",
                      day: "2-digit",
                      hour: "2-digit",
                      minute: "2-digit",
                      second: "2-digit",
                      hour12: false,
                    }
                  )}
                </td>
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                  {movingLogs[vItem.index]?.lat == null ||
                  movingLogs[vItem.index]?.lon == null
                    ? "—"
                    : `${movingLogs[vItem.index]?.lat?.toFixed(
                        5
                      )}, ${movingLogs[vItem.index]?.lon?.toFixed(5)}`}
                </td>
                {movingLogs[vItem.index]?.speed == null ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    —
                  </td>
                ) : (movingLogs[vItem.index]?.speed ?? 0) < 0 ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold font-mono tabular-nums whitespace-nowrap">
                    {movingLogs[vItem.index]?.speed?.toFixed(2)} m/s (
                    {toKMH(movingLogs[vItem.index]?.speed ?? 0).toFixed(2)}{" "}
                    km/h)
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    {movingLogs[vItem.index]?.speed?.toFixed(2)} m/s (
                    {toKMH(movingLogs[vItem.index]?.speed ?? 0).toFixed(2)}{" "}
                    km/h)
                  </td>
                )}
                {movingLogs[vItem.index]?.accuracy == null ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    —
                  </td>
                ) : (movingLogs[vItem.index]?.accuracy ?? 0) >
                  BAD_ACCURACY_THRESHOLD ? (
                  <td className="p-2 border border-gray-200 dark:border-white/15 text-red-600 font-bold font-mono  tabular-nums whitespace-nowrap">
                    {movingLogs[vItem.index]?.accuracy?.toFixed(2)}m
                  </td>
                ) : (
                  <td className="p-2 border border-gray-200 dark:border-white/15 font-mono tabular-nums whitespace-nowrap">
                    {movingLogs[vItem.index]?.accuracy?.toFixed(2)}m
                  </td>
                )}
                <td className="p-2 border border-gray-200 dark:border-white/15 font-mono whitespace-nowrap">
                  {movingLogs[vItem.index]?.device}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
);
