import type { UnlistenFn } from "@tauri-apps/api/event";
import { useAtom } from "jotai";
import uniqBy from "lodash/uniqBy";
import { useCallback, useEffect, useState } from "react";
import { telemetryListAtom } from "~/atoms/telemetryItem";
import {
  type ErrorData,
  type LocationData,
  type LogData,
  registerTelemetryListener,
} from "~/domain/commands";
import { isLocalServerEnabledAsync } from "~/utils/server";

export const useTelemetry = () => {
  const [isLocalServerAvailable, setIsLocalServerAvailable] = useState(false);
  const [error, setError] = useState<ErrorData | null>(null);
  const [consoleLogs, setConsoleLogs] = useState<LogData[]>([]);
  const [telemetryList, setTelemetryList] = useAtom(telemetryListAtom);

  const handleLocationUpdate = useCallback(
    (data: LocationData) => {
      setTelemetryList((prev) => {
        const newList = uniqBy([...prev, { ...data }], "id");
        // 最大1,000件に制限してメモリ使用量を管理
        return newList.slice(-1000);
      });
    },
    [setTelemetryList]
  );

  useEffect(() => {
    const updateServerAvailabilityAsync = async () => {
      setIsLocalServerAvailable(await isLocalServerEnabledAsync());
    };
    updateServerAvailabilityAsync();
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn;
    registerTelemetryListener({
      onLocationUpdate: handleLocationUpdate,
      onError: (err) => setError(err),
      onLog: (log) =>
        setConsoleLogs((prev) => {
          const newLogs = uniqBy([...prev, log], "id");
          // コンソールログも最大1,000件に制限
          return newLogs.slice(-1000);
        }),
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [handleLocationUpdate]);

  return {
    telemetryList,
    error,
    consoleLogs,
    isLocalServerAvailable,
  };
};
