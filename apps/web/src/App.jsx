import { useEffect, useRef, useState } from "react";
import WorldView, { createController } from "./WorldView.jsx";
import { decode } from "./render/protocol.js";
import { speciesCss } from "./render/WorldRenderer.js";

const PROTOCOL_VERSION = 1;

// the readout refreshes ten times a second whatever the epoch rate is
const HUD_INTERVAL = 100;

// the server owns the runs. when one ends its socket closes, so tuning back in
// is how the next one is picked up - after a counted breather, so the ending of
// one run is readable before the next world wipes it away.
const RECONNECT_SECONDS = 10;

export default function App() {
  const [start, setStart] = useState(null);
  const [status, setStatus] = useState("connecting");
  const [failure, setFailure] = useState(null);
  const [done, setDone] = useState(null);
  const [hud, setHud] = useState({ report: null, fps: 0, frameP95: 0 });

  const report = useRef(null);
  const species = useRef([]);
  const lastSnapshot = useRef(null);
  const controller = useRef(null);
  controller.current ??= createController();

  // per-epoch reports and per-frame counters both land in refs; React reads
  // them on a fixed cadence and never from the render loop
  useEffect(() => {
    const id = setInterval(() => {
      const stats = controller.current.stats();
      setHud({
        report: report.current,
        fps: stats?.frameP50 ? Math.round(1000 / stats.frameP50) : 0,
        frameP95: stats?.frameP95 ?? 0,
      });
    }, HUD_INTERVAL);
    return () => clearInterval(id);
  }, []);

  // one socket at a time, opened on mount and reopened whenever it drops.
  // nothing here asks the server for anything.
  useEffect(() => {
    let socket = null;
    let retry = null;
    let closed = false;

    function connect() {
      setStatus("connecting");
      const ws = new WebSocket(`${location.origin.replace("http", "ws")}/ws`);
      ws.binaryType = "arraybuffer";
      socket = ws;

      const live = () => socket === ws && !closed;

      ws.onmessage = (e) => {
        if (!live()) return;
        try {
          if (typeof e.data === "string") text(JSON.parse(e.data));
          else binary(e.data);
        } catch (err) {
          setFailure(err.message);
          setStatus("protocol error");
          ws.close();
        }
      };
      ws.onerror = () => {
        if (live()) setStatus("no server - is `npm run server` up?");
      };
      ws.onclose = () => {
        if (closed) return;
        let left = RECONNECT_SECONDS;
        const show = () =>
          setStatus((s) =>
            s === "protocol error" ? s : `next run in ${left}s`,
          );
        show();
        retry = setInterval(() => {
          left -= 1;
          if (left > 0) return show();
          clearInterval(retry);
          connect();
        }, 1000);
      };

      function text(msg) {
        if (msg.type === "config") {
          if (msg.protocol_version !== PROTOCOL_VERSION) {
            throw new Error(
              `server speaks protocol ${msg.protocol_version}, this build speaks ${PROTOCOL_VERSION}`,
            );
          }
          // a new run: drop everything the last one left behind
          species.current = msg.species;
          report.current = null;
          lastSnapshot.current = null;
          controller.current.reset();
          setDone(null);
          setFailure(null);
          setStart(msg);
          setStatus("watching");
        }
        if (msg.type === "epoch") report.current = msg.report;
        if (msg.type === "error") throw new Error(msg.message);
        if (msg.type === "done") {
          setDone(msg);
          // the terminal snapshot arrives before `done`, so a run that ends
          // without one ended early rather than finishing
          setStatus(
            lastSnapshot.current === msg.epochs ? "complete" : "ended early",
          );
        }
      }

      function binary(buffer) {
        const message = decode(buffer, {
          speciesCount: species.current.length,
        });
        if (message.kind === "world") {
          controller.current.setWorld(message, species.current);
        } else {
          lastSnapshot.current = message.epoch;
          controller.current.setSnapshot(message);
        }
      }
    }

    connect();
    return () => {
      closed = true;
      clearInterval(retry);
      socket?.close();
    };
  }, []);

  const last = hud.report;
  const cards = last?.species ?? start?.species ?? [];

  return (
    <div className="fixed inset-0 bg-neutral-950 font-mono text-neutral-200">
      <WorldView controller={controller.current} />

      {/* nothing here is interactive, so nothing here may sit between the
          viewer and the world */}
      <div className="pointer-events-none absolute bottom-4 left-4 max-w-[min(22rem,calc(100vw-2rem))] rounded border border-neutral-800/80 bg-neutral-950/70 p-3 text-xs backdrop-blur">
        <div className="flex items-baseline gap-2">
          <span className="text-sm font-bold text-emerald-400">ecosym</span>
          <span className="text-neutral-400">{status}</span>
          {start && <span className="text-neutral-600">{start.engine}</span>}
        </div>

        {failure && <p className="mt-1 text-amber-400">{failure}</p>}

        {last && (
          <div className="mt-2 grid grid-cols-2 gap-x-4 gap-y-0.5 tabular-nums text-neutral-400">
            <Row label="epoch" value={last.epoch.toLocaleString()} />
            <Row label="population" value={last.population.toLocaleString()} />
            <Row
              label="biomass"
              value={Math.round(last.biomass).toLocaleString()}
            />
            <Row
              label="fps"
              value={`${hud.fps} · p95 ${hud.frameP95.toFixed(1)}ms`}
            />
          </div>
        )}

        {cards.length > 0 && (
          <div className="mt-2 space-y-0.5">
            {/* species stay in the order the server sent them */}
            {cards.map((s, i) => (
              <div key={s.id} className="flex items-baseline gap-2">
                <span
                  className="h-2 w-2 rounded-full"
                  style={{ background: speciesCss(i) }}
                />
                <span className="text-neutral-300">{s.name}</span>
                <span className="ml-auto tabular-nums text-neutral-100">
                  {(s.population ?? 0).toLocaleString()}
                </span>
                {s.births !== undefined && (
                  <span className="w-24 text-right tabular-nums text-neutral-600">
                    +{s.births} / -{s.deaths}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}

        {start && (
          <p className="mt-2 text-neutral-600">
            world {start.world.width}x{start.world.height} ·{" "}
            {start.world.habitable_tiles.toLocaleString()} habitable · seed{" "}
            {start.seed_hex}
          </p>
        )}
      </div>

      {/* the run's obituary, and the only thing that ever covers the world. it
          arrives with `done` and leaves on its own when the next run's config
          lands, so the transition between runs is something you can read */}
      {done && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">
          <div className="w-80 max-w-full animate-[outcome-in_400ms_ease-out] rounded border border-neutral-800/80 bg-neutral-950/85 p-4 text-xs backdrop-blur motion-reduce:animate-none">
            <div className="flex items-baseline justify-between text-neutral-600">
              <span>{done.epochs.toLocaleString()} epochs</span>
              <span>{status}</span>
            </div>

            <p className="mt-2 text-sm text-neutral-100">
              {winnerLine(done.outcome)}
            </p>

            {/* same order as everywhere else, so a colour means one species */}
            <div className="mt-3 space-y-0.5 tabular-nums">
              {done.outcome.species.map((s, i) => (
                <div key={s.id} className="flex items-baseline gap-2">
                  <span
                    className="h-2 w-2 rounded-full"
                    style={{ background: speciesCss(i) }}
                  />
                  <span className="text-neutral-300">{s.name}</span>
                  <span className="ml-auto text-neutral-600">
                    {s.initial.toLocaleString()} &rarr;
                  </span>
                  <span className="w-12 text-right text-neutral-100">
                    {s.final_population.toLocaleString()}
                  </span>
                </div>
              ))}
            </div>

            {/* one identifier per line: a u64 seed is long enough to wrap a
                shared line mid-phrase */}
            <div className="mt-3 text-neutral-600">
              <p>seed {start?.seed_hex}</p>
              <p>
                digest <span className="text-emerald-400">{done.digest}</span>
              </p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function Row({ label, value }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-neutral-600">{label}</span>
      <span className="ml-auto text-neutral-100">{value}</span>
    </div>
  );
}

function winnerLine(outcome) {
  if (!outcome) return "";
  const name = (id) =>
    outcome.species.find((s) => s.id === id)?.name ?? `species ${id}`;
  if (outcome.winner === "None") return "no winner, everything died";
  if (outcome.winner.Species !== undefined)
    return `winner ${name(outcome.winner.Species)}`;
  return `tie between ${outcome.winner.Tie.map(name).join(", ")}`;
}
