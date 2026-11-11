import { useEffect, useRef, useState } from "react";
import type { AppState, ClientMsg, ServerMsg } from "./generated/protocol";

/** `reconnecting` only after a connection was open and dropped. */
export type Conn = "connecting" | "open" | "reconnecting";

const MIN_DELAY = 250;
const MAX_DELAY = 5000;

/** The core's last state over `/ws`, reconnecting with backoff when the socket drops. */
export function useAppSocket() {
  const [state, setState] = useState<AppState | null>(null);
  const [conn, setConn] = useState<Conn>("connecting");
  const socket = useRef<WebSocket | null>(null);

  useEffect(() => {
    let delay = MIN_DELAY;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const open = () => {
      const ws = new WebSocket(`ws://${location.host}/ws`);
      socket.current = ws;
      ws.onopen = () => {
        delay = MIN_DELAY;
        setConn("open");
      };
      ws.onmessage = (e) => {
        const msg = JSON.parse(e.data as string) as ServerMsg;
        if (msg.type === "state") setState(msg.state);
      };
      ws.onclose = () => {
        socket.current = null;
        if (disposed) return;
        setConn((c) => (c === "connecting" ? c : "reconnecting"));
        timer = setTimeout(open, delay);
        delay = Math.min(delay * 2, MAX_DELAY);
      };
    };
    open();
    return () => {
      disposed = true;
      clearTimeout(timer);
      socket.current?.close();
    };
  }, []);

  const send = (msg: ClientMsg) => {
    if (socket.current?.readyState === WebSocket.OPEN) socket.current.send(JSON.stringify(msg));
  };
  return { state, conn, send };
}
