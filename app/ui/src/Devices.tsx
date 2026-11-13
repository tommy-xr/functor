import { useEffect, useRef, useState } from "react";
import type { Attached, Device, Devices, PairRequest } from "./generated/protocol";
import type { Send } from "./socket";
import {
  CABLE_ROW,
  cableFor,
  cableOnly,
  cableWords,
  clock,
  details,
  fixFor,
  pairedWith,
  secondsLeft,
  spacedCode,
  WORDS,
} from "./devices-model";
import { relativeTime } from "./tasks-model";
import { useNow } from "./TasksPane";

/**
 * Rail → Devices: a card per paired device (and per request, and per
 * headset on a USB cable that has no card), each with its state's one fix.
 */
export function DevicesPane({ devices, online, send }: { devices: Devices | undefined; online: boolean; send: Send }) {
  const now = useNow(1000);
  const list = devices?.list ?? [];
  const requests = devices?.requests ?? [];
  const attached = devices?.attached ?? [];
  const loose = devices ? cableOnly(devices) : [];
  return (
    <aside className="pane pane-devices" aria-label="Devices">
      <h2 className="pane-title">Devices</h2>
      {requests.map((r) => (
        <RequestCard key={r.request} request={r} now={now} online={online} send={send} />
      ))}
      {list.map((d) => (
        <DeviceCard key={d.id} device={d} cable={cableFor(d, attached)} now={now} online={online} send={send} />
      ))}
      {loose.map((a) => (
        <CableCard key={a.serial} cable={a} online={online} send={send} />
      ))}
      {list.length === 0 && requests.length === 0 && loose.length === 0 && (
        <NoDevices listening={devices?.listening ?? null} adb={devices?.adb ?? false} />
      )}
    </aside>
  );
}

function NoDevices({ listening, adb }: { listening: string | null; adb: boolean }) {
  return (
    <section className="card device-card" data-testid="devices-empty">
      <div className="device-head">
        <span className="device-name">No headset yet</span>
        <span className="pill">{WORDS.not_paired}</span>
      </div>
      {listening ? (
        <>
          <p>Open Functor in the headset on the same Wi-Fi. It finds this computer and shows a code; you allow it here.</p>
          <p className="hint">Nothing to type in the headset.</p>
          <p className="hint">
            {adb
              ? "Or plug it into this computer with a USB cable: it pairs with no code."
              : "To pair over a USB cable instead, install Android platform-tools (adb)."}
          </p>
        </>
      ) : (
        <p className="hint">The device link is off, so headsets cannot find this computer. Start the app without --no-device-link.</p>
      )}
    </section>
  );
}

/**
 * The decision on a request, the same in its card and in the prompt: the
 * warning when it claims a paired headset's id, Not mine, and Allow (off
 * once allowed or expired).
 */
function PairDecision({ request: r, now, online, send }: { request: PairRequest; now: number; online: boolean; send: Send }) {
  const expired = secondsLeft(r, now) === 0;
  return (
    <>
      {r.repair && <p className="pair-warn">It claims the id of a headset already paired. Allow only if you reinstalled its app.</p>}
      <div className="actions device-actions">
        <button type="button" className="button" disabled={!online} onClick={() => send({ type: "deny_pairing", request: r.request })}>
          Not mine
        </button>
        <button
          type="button"
          className="button button-primary"
          disabled={!online || r.allowed || expired}
          onClick={() => send({ type: "allow_pairing", request: r.request })}
        >
          {r.allowed ? "Waiting for the headset…" : "Allow"}
        </button>
      </div>
    </>
  );
}

/** What the request waits on, for the user. */
function requestStatus(r: PairRequest, now: number): string {
  if (secondsLeft(r, now) === 0) return "This code has expired.";
  return r.allowed ? "Allowed. Now confirm the code in the headset." : "Check the same code is showing in the headset.";
}

function RequestCard({ request: r, now, online, send }: { request: PairRequest; now: number; online: boolean; send: Send }) {
  return (
    <section className="card device-card" data-testid="device-card" data-state="asking_to_pair" aria-label={`${r.name}, ${WORDS.asking_to_pair}`}>
      <div className="device-head">
        <span className="device-name">{r.name}</span>
        <span className="pill pill-hs-asking_to_pair">{WORDS.asking_to_pair}</span>
      </div>
      <dl className="rows">
        <div>
          <dt>Found on</dt>
          <dd>Wi-Fi</dd>
        </div>
        <div>
          <dt>Code</dt>
          <dd className="device-code">{spacedCode(r.code)}</dd>
        </div>
        <div>
          <dt>Expires in</dt>
          <dd className="mono">{clock(secondsLeft(r, now))}</dd>
        </div>
      </dl>
      <p className="hint">{requestStatus(r, now)}</p>
      <PairDecision request={r} now={now} online={online} send={send} />
    </section>
  );
}

/**
 * "Pair over the cable": one click, no code. A headset paired with another
 * desktop asks first, in place, before its pairing is replaced.
 */
function CablePair({ cable: a, online, send }: { cable: Attached; online: boolean; send: Send }) {
  const [confirming, setConfirming] = useState(false);
  const held = pairedWith(a);
  const other = held?.with === "other" ? held.name : null;
  const pair = (replace: boolean) => {
    setConfirming(false);
    send({ type: "pair_by_cable", serial: a.serial, replace });
  };
  if (a.pairing) {
    return (
      <div className="actions device-actions">
        <button type="button" className="button button-primary" disabled>
          Pairing…
        </button>
      </div>
    );
  }
  if (!a.pairable) return null;
  if (confirming && other) {
    return (
      <div className="task-confirm device-confirm" role="group" aria-label={`Replace the pairing with ${other}`}>
        <p>Pair it with this computer instead? {other} will no longer reach it.</p>
        <div className="actions device-actions">
          <button type="button" className="button" onClick={() => setConfirming(false)}>
            Keep
          </button>
          <button type="button" className="button button-danger" disabled={!online} onClick={() => pair(true)}>
            Replace
          </button>
        </div>
      </div>
    );
  }
  return (
    <div className="actions device-actions">
      <button
        type="button"
        className="button button-primary"
        disabled={!online}
        onClick={() => (other ? setConfirming(true) : pair(false))}
      >
        Pair over the cable
      </button>
    </div>
  );
}

/** A headset on a USB cable with no card of its own: not paired here. */
function CableCard({ cable: a, online, send }: { cable: Attached; online: boolean; send: Send }) {
  const { line, fix } = cableWords(a);
  return (
    <section className="card device-card" data-testid="cable-card" data-state={a.state.state} aria-label={`${a.name}, on the USB cable`}>
      <div className="device-head">
        <span className="device-name">{a.name}</span>
        <span className="pill pill-hs-not_paired">{WORDS.not_paired}</span>
      </div>
      <dl className="rows">
        <div>
          <dt>Found on</dt>
          <dd>USB cable</dd>
        </div>
      </dl>
      <p>{line}</p>
      {fix && <p className="hint">{fix}</p>}
      <CablePair cable={a} online={online} send={send} />
    </section>
  );
}

function DeviceCard({
  device: d,
  cable,
  now,
  online,
  send,
}: {
  device: Device;
  /** The headset on a USB cable, if it is. */
  cable: Attached | null;
  now: number;
  online: boolean;
  send: Send;
}) {
  const [confirming, setConfirming] = useState(false);
  const word = d.state.state;
  const fix = fixFor(d, cable);
  const rows: [string, string][] =
    d.state.state === "connected"
      ? details(d)
      : [
          ["Last seen", relativeTime(d.last_seen, now) || "never"],
          ["Headset app", d.app_version ?? "—"],
        ];
  if (cable) rows.push(["Cable", CABLE_ROW[cable.state.state]]);
  const reason =
    d.state.state === "cannot_reach" ? `Last error: ${d.state.reason}` : d.state.state === "needs_update" ? d.state.reason : null;
  // On the cable, its own file says whether it still holds this pairing.
  const held = cable && pairedWith(cable);
  const lost =
    !d.paired || !held || held.with === "this"
      ? null
      : held.with === "other"
        ? `The headset is now paired with ${held.name}.`
        : "The headset no longer holds this pairing.";
  return (
    <section className="card device-card" data-testid="device-card" data-state={word} aria-label={`${d.name}, ${WORDS[word]}`}>
      <div className="device-head">
        <span className="device-name">{d.name}</span>
        <span className={`pill pill-hs-${word}`}>{WORDS[word]}</span>
      </div>
      <dl className="rows">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd className={k === "Build" ? "mono" : undefined}>{v}</dd>
          </div>
        ))}
      </dl>
      {reason && <p className="device-reason">{reason}</p>}
      {lost && <p className="device-reason">{lost}</p>}
      {/* Pairing again over the cable is the fix then. */}
      {fix && !lost && <p className="hint">{fix}</p>}
      {d.paired && cable && <CablePair cable={cable} online={online} send={send} />}
      {d.paired &&
        (confirming ? (
          <div className="task-confirm device-confirm" role="group" aria-label={`Forget ${d.name}`}>
            <p>Forget {d.name}? It will need to pair again before it can connect.</p>
            <div className="actions device-actions">
              <button type="button" className="button" onClick={() => setConfirming(false)}>
                Keep
              </button>
              <button
                type="button"
                className="button button-danger"
                disabled={!online}
                onClick={() => send({ type: "forget_device", device: d.id })}
              >
                Forget
              </button>
            </div>
          </div>
        ) : (
          <button type="button" className="device-forget" disabled={!online} onClick={() => setConfirming(true)}>
            Forget this headset
          </button>
        ))}
    </section>
  );
}

/**
 * "A Quest 3 wants to pair": one request, until the user decides, puts it
 * off ("Later"), the headset cancels or the code expires. Only its buttons
 * close it: no Escape, no click outside. Focus goes to the dialog itself,
 * never to Allow, so a stray Enter allows nothing; it returns where it was.
 */
export function PairPrompt({
  request: r,
  waiting,
  online,
  send,
  onLater,
}: {
  request: PairRequest;
  /** Requests queued behind this one. */
  waiting: number;
  online: boolean;
  send: Send;
  /** Put it off; the title pill brings it back. */
  onLater: () => void;
}) {
  const now = useNow(1000);
  const left = secondsLeft(r, now);
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => dialog.current?.focus(), [r.request]);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    // After the app behind stops being inert, which happens as this closes.
    return () => void setTimeout(() => before?.isConnected && before.focus());
  }, []);
  return (
    // A click outside neither closes it nor takes the focus away.
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && e.preventDefault()}>
      <div
        ref={dialog}
        className="modal pair-modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="pair-title"
        aria-describedby="pair-hint"
        tabIndex={-1}
        data-testid="pair-prompt"
      >
        <h2 id="pair-title">{r.name} wants to pair</h2>
        <p id="pair-hint">It found this computer on your Wi-Fi. Allow it only if the headset shows the same code.</p>
        <div className="pair-code" data-testid="pair-code" aria-label={`Code ${r.code.split("").join(" ")}`}>
          {spacedCode(r.code)}
        </div>
        {left > 0 && !r.allowed && <p className="hint">Expires in {clock(left)}</p>}
        {/* Announced: only what changes the decision, not the countdown. */}
        <p className="hint" aria-live="polite">
          {left === 0 || r.allowed ? requestStatus(r, now) : ""}
        </p>
        <PairDecision request={r} now={now} online={online} send={send} />
        <button type="button" className="button button-quiet" onClick={onLater}>
          Decide later
        </button>
        <p className="hint">
          Pairing lets this headset run your projects. You can forget it later in Devices.
          {waiting > 0 && ` ${waiting} more ${waiting === 1 ? "request is" : "requests are"} waiting.`}
        </p>
      </div>
    </div>
  );
}
