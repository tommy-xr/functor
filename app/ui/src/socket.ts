import { useCallback, useEffect, useRef, useState } from "react";
import type { AppState, ClientMsg, Item, ServerMsg } from "./generated/protocol";
import { applyItem } from "./chat-model";

export type Send = (msg: ClientMsg) => boolean;

/** `reconnecting` only after a connection was open and dropped. */
export type Conn = "connecting" | "open" | "reconnecting";

/** A user message the core accepted (its item arrived); `seq` tells repeats apart. */
export type Accepted = { task: number; text: string; seq: number };
export type Notice = { reason: string; seq: number };
export type DiffMsg = Extract<ServerMsg, { type: "diff" }>;
export type FileMsg = Extract<ServerMsg, { type: "file" }>;

/** `files`' key for `path` of `project`. */
export const fileKey = (project: string, path: string) => `${project}\n${path}`;

const MIN_DELAY = 250;
const MAX_DELAY = 5000;

/**
 * The core's last state over `/ws`, reconnecting with backoff when the
 * socket drops, and the transcripts fetched so far. A transcript is fetched
 * once per connection (`fetchTranscript`), then kept current by `Item`s; after
 * a reconnect the old one shows until its fetch replaces it.
 */
export function useAppSocket() {
  const [state, setState] = useState<AppState | null>(null);
  const [conn, setConn] = useState<Conn>("connecting");
  const [transcripts, setTranscripts] = useState<ReadonlyMap<number, Item[]>>(new Map());
  const [accepted, setAccepted] = useState<Accepted | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  // The last diff of each task asked for (`fetchDiff`).
  const [diffs, setDiffs] = useState<ReadonlyMap<number, DiffMsg>>(new Map());
  // The last text of each file asked for (`fetchFile`), by `fileKey`.
  const [files, setFiles] = useState<ReadonlyMap<string, FileMsg>>(new Map());
  const socket = useRef<WebSocket | null>(null);
  // Asked for on this connection.
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
          case "diff":
            setDiffs((all) => new Map(all).set(msg.task, msg));
            break;
          case "file":
            setFiles((all) => {
              const key = fileKey(msg.project, msg.path);
              const had = all.get(key);
              // A refresh that changed nothing keeps the views as they are.
              if (had?.text === msg.text && had?.error === msg.error) return all;
              return new Map(all).set(key, msg);
            });
            break;
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

  // Stable, so effects and listeners that use it are not redone per message.
  const send: Send = useCallback((msg) => {
    if (socket.current?.readyState !== WebSocket.OPEN) return false;
    socket.current.send(JSON.stringify(msg));
    return true;
  }, []);
  const fetchTranscript = (task: number) => {
    if (requested.current.has(task) || !send({ type: "get_transcript", task })) return;
    requested.current.add(task);
  };
  const fetchDiff = (task: number) => send({ type: "get_diff", task });
  const fetchFile = (path: string) => send({ type: "get_file", path });
  return { state, conn, send, transcripts, fetchTranscript, accepted, notice, diffs, fetchDiff, files, fetchFile };
}
