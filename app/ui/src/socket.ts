import { useCallback, useEffect, useRef, useState } from "react";
import type { AppState, ClientMsg, Item, Mark, NewProjectRequest, ServerMsg, Transport } from "./generated/protocol";
import { applyItem } from "./chat-model";

export type Send = (msg: ClientMsg) => boolean;

/** `reconnecting` only after a connection was open and dropped. */
export type Conn = "connecting" | "open" | "reconnecting";

/** A user message the core accepted (its item arrived); `seq` tells repeats apart. */
export type Accepted = { task: number; text: string; seq: number };
export type Notice = { reason: string; seq: number };
export type DiffMsg = Extract<ServerMsg, { type: "diff" }>;
export type FileMsg = Extract<ServerMsg, { type: "file" }>;
export type DirsMsg = Extract<ServerMsg, { type: "dirs" }>;
export type PlanMsg = Extract<ServerMsg, { type: "new_project_plan" }>;

/** `files`' key for `path` of `project`. */
export const fileKey = (project: string, path: string) => `${project}\n${path}`;

const MIN_DELAY = 250;
const MAX_DELAY = 5000;

/**
 * This launch's token for `/ws`, kept in memory only: set by the app window,
 * or the `#token=` the `--headless` URL carries, which is then dropped from
 * the address bar and history entry. Without it the server refuses the socket.
 */
const TOKEN = takeToken();

/** False after a reload in a browser, say: the page can then never connect. */
export const HAS_TOKEN = TOKEN !== "";

function takeToken(): string {
  const injected = (window as { functorToken?: string }).functorToken;
  if (injected) return injected;
  const token = /^#token=([0-9a-zA-Z_-]+)$/.exec(location.hash)?.[1] ?? "";
  if (token) history.replaceState(null, "", location.pathname + location.search);
  return token;
}

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
  // The last directory suggestions asked for (`listDirs`).
  const [dirs, setDirs] = useState<DirsMsg | null>(null);
  // The last new-project plan asked for (`checkProject`).
  const [plan, setPlan] = useState<PlanMsg | null>(null);
  // The active session's clock and marks, sent apart from the state.
  const [transport, setTransport] = useState<Transport | null>(null);
  const [marks, setMarks] = useState<Mark[]>([]);
  // The plan asked for last: an older one answered late is dropped.
  const planAsked = useRef("");
  const socket = useRef<WebSocket | null>(null);
  // The open folder's root: files are keyed by project path, which another
  // folder's project may share.
  const root = useRef<string | null>(null);
  // Asked for on this connection.
  const requested = useRef(new Set<number>());

  useEffect(() => {
    let delay = MIN_DELAY;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    let seq = 0;
    const open = () => {
      const ws = new WebSocket(`ws://${location.host}/ws?token=${encodeURIComponent(TOKEN)}`);
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
            if (root.current !== null && root.current !== msg.state.repo_root) setFiles(new Map());
            root.current = msg.state.repo_root;
            setState(msg.state);
            break;
          case "dirs":
            setDirs(msg);
            break;
          case "new_project_plan":
            if (JSON.stringify(msg.request) === planAsked.current) setPlan(msg);
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
            // Read for a folder open before this one.
            if (msg.root !== root.current) break;
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
          case "transport":
            setTransport(msg.transport);
            break;
          case "marks":
            setMarks(msg.marks);
            break;
        }
      };
      ws.onclose = () => {
        socket.current = null;
        // A new connection is sent them afresh, if any.
        setTransport(null);
        setMarks([]);
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
  const listDirs = (path: string) => send({ type: "list_dirs", path });
  const checkProject = (request: NewProjectRequest) => {
    planAsked.current = JSON.stringify(request);
    send({ type: "check_new_project", request });
  };
  return {
    state,
    conn,
    send,
    transcripts,
    fetchTranscript,
    accepted,
    notice,
    diffs,
    fetchDiff,
    files,
    fetchFile,
    dirs,
    listDirs,
    plan,
    checkProject,
    transport,
    marks,
  };
}
