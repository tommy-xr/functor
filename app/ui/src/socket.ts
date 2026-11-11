import { useEffect, useRef, useState } from "react";
import type { AppState, ClientMsg, Item, ServerMsg } from "./generated/protocol";
import { applyItem } from "./chat-model";

/** `reconnecting` only after a connection was open and dropped. */
export type Conn = "connecting" | "open" | "reconnecting";

/** A user message the core accepted (its item arrived); `seq` tells repeats apart. */
export type Accepted = { task: number; text: string; seq: number };
export type Notice = { reason: string; seq: number };

const MIN_DELAY = 250;
const MAX_DELAY = 5000;

/**
 * The core's last state over `/ws`, reconnecting with backoff when the
 * socket drops, and the transcripts fetched so far. A transcript is fetched
 * once (`fetchTranscript`), then kept current by `Item`s; a reconnect drops
 * them all so each is fetched again.
 */
export function useAppSocket() {
  const [state, setState] = useState<AppState | null>(null);
  const [conn, setConn] = useState<Conn>("connecting");
  const [transcripts, setTranscripts] = useState<ReadonlyMap<number, Item[]>>(new Map());
  const [accepted, setAccepted] = useState<Accepted | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const socket = useRef<WebSocket | null>(null);
  // Asked for and not yet dropped; cleared with the transcripts on reconnect.
  const requested = useRef(new Set<number>());

  useEffect(() => {
    let delay = MIN_DELAY;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    let seq = 0;
    const open = () => {
      const ws = new WebSocket(`ws://${location.host}/ws`);
      socket.current = ws;
      ws.onopen = () => {
        delay = MIN_DELAY;
        requested.current.clear();
        setTranscripts(new Map());
        setConn("open");
      };
      ws.onmessage = (e) => {
        const msg = JSON.parse(e.data as string) as ServerMsg;
        switch (msg.type) {
          case "state":
            setState(msg.state);
            break;
          case "transcript":
            setTranscripts((all) => new Map(all).set(msg.task, msg.items));
            break;
          case "item": {
            const { task, index, item } = msg;
            if (item.kind === "user") setAccepted({ task, text: item.text, seq: ++seq });
            setTranscripts((all) => {
              const had = all.get(task);
              // Item 0 is only ever sent when its task is created: the whole transcript.
              const next = had === undefined && index === 0 ? [item] : applyItem(had, index, item);
              if (next === undefined) return all;
              const copy = new Map(all);
              if (next === "refetch") {
                copy.delete(task);
                requested.current.delete(task);
              } else copy.set(task, next);
              return copy;
            });
            break;
          }
          case "rejected":
            setNotice({ reason: msg.reason, seq: ++seq });
            break;
        }
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
    if (socket.current?.readyState !== WebSocket.OPEN) return false;
    socket.current.send(JSON.stringify(msg));
    return true;
  };
  const fetchTranscript = (task: number) => {
    if (requested.current.has(task) || !send({ type: "get_transcript", task })) return;
    requested.current.add(task);
  };
  return { state, conn, send, transcripts, fetchTranscript, accepted, notice };
}
