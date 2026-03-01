import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";

const SSE_EVENT_TYPES = ["announce", "withdraw", "session_up", "session_down"];
const RECONNECT_DELAY_MS = 5000;

export function useEventStream() {
  const queryClient = useQueryClient();
  const retryTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    let source: EventSource | null = null;

    function connect() {
      source = new EventSource("/api/v1/events/stream");

      for (const type of SSE_EVENT_TYPES) {
        source.addEventListener(type, () => {
          queryClient.invalidateQueries({ queryKey: ["summary"] });
          queryClient.invalidateQueries({ queryKey: ["peers"] });
        });
      }

      source.onerror = () => {
        // Close the failed connection and retry after a delay.
        // EventSource's built-in reconnect can be aggressive; we control
        // the cadence ourselves to avoid console spam.
        source?.close();
        source = null;
        retryTimer.current = setTimeout(connect, RECONNECT_DELAY_MS);
      };
    }

    connect();

    return () => {
      clearTimeout(retryTimer.current);
      source?.close();
    };
  }, [queryClient]);
}
