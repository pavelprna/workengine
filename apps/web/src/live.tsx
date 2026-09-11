import { useQueryClient } from "@tanstack/react-query";
import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react";
import type { WorkEvent } from "./api/client";

export type LiveConnection = "connecting" | "live" | "reconnecting";

const LiveConnectionContext = createContext<LiveConnection>("connecting");

export function LiveEventsProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [connection, setConnection] = useState<LiveConnection>("connecting");

  useEffect(() => {
    const source = new EventSource("/api/v0/events/stream");
    const dirtyWorkIds = new Set<string>();
    let flushTimer: number | undefined;

    function flush() {
      flushTimer = undefined;
      void queryClient.invalidateQueries({ queryKey: ["overview"] });
      void queryClient.invalidateQueries({ queryKey: ["works"] });
      for (const workId of dirtyWorkIds) {
        void queryClient.invalidateQueries({ queryKey: ["work", workId] });
        void queryClient.invalidateQueries({ queryKey: ["events", workId] });
      }
      dirtyWorkIds.clear();
    }

    source.onopen = () => setConnection("live");
    source.onerror = () => setConnection("reconnecting");
    source.addEventListener("workengine.event", (message) => {
      const event = JSON.parse(
        (message as MessageEvent<string>).data,
      ) as WorkEvent;
      dirtyWorkIds.add(event.workId);
      if (flushTimer === undefined) flushTimer = window.setTimeout(flush, 120);
    });

    return () => {
      source.close();
      if (flushTimer !== undefined) window.clearTimeout(flushTimer);
    };
  }, [queryClient]);

  return (
    <LiveConnectionContext.Provider value={connection}>
      {children}
    </LiveConnectionContext.Provider>
  );
}

export function useLiveConnection() {
  return useContext(LiveConnectionContext);
}
