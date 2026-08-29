import { useCallback, useEffect, useRef, useState } from "react";
import AccountPanel from "./AccountPanel.jsx";
import BetPanel from "./BetPanel.jsx";
import RunResult from "./RunResult.jsx";
import WorldView, { createController } from "./WorldView.jsx";
import { api } from "./game/api.js";
import { fromAnotherRun, initialMarket, reduceMarket } from "./game/market.js";
import { decode } from "./render/protocol.js";
import { speciesCss } from "./render/WorldRenderer.js";

const PROTOCOL_VERSION = 1;

// the readout refreshes ten times a second whatever the epoch rate is
const HUD_INTERVAL = 100;

// the server owns the run and keeps going without us, so a dropped socket is
// only ever our problem to fix
const RECONNECT_SECONDS = 3;

export default function App() {
  const [start, setStart] = useState(null);
  const [status, setStatus] = useState("connecting");
  const [connected, setConnected] = useState(false);
  const [failure, setFailure] = useState(null);
  const [done, setDone] = useState(null);
  const [hud, setHud] = useState({ report: null, fps: 0, frameP95: 0 });
  const [game, setGame] = useState(initialMarket);
  const [account, setAccount] = useState(null);
  const [bet, setBet] = useState(null);
  // bumped whenever the socket has to start again: a bootstrap that did not
  // add up, or signing in and out, which is what re-authenticates it
  const [socketKey, setSocketKey] = useState(0);

  const report = useRef(null);
  const species = useRef([]);
  const lastSnapshot = useRef(null);
  const runId = useRef(null);
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

  // an account is only ever what the server last said it was. a newer
  // revision may not be overwritten by an older fetch.
  const refreshAccount = useCallback(async () => {
    try {
      const next = await api.me();
      setAccount((prev) => (prev && next.revision < prev.revision ? prev : next));
    } catch (e) {
      if (e.status === 401) setAccount(null);
    }
  }, []);

  const refreshMarket = useCallback(async () => {
    try {
      const market = await api.market();
      setBet(market.bet ?? null);
      setGame((g) => reduceMarket(g, { type: "market_fetched", market }));
    } catch {
      // the socket carries the market too; a failed fetch is not fatal
    }
  }, []);

  useEffect(() => {
    refreshAccount();
    refreshMarket();
  }, [refreshAccount, refreshMarket]);

  // whose bet this is changes with the market and with settlement, and both
  // arrive on the socket rather than from anything this page did
  const marketId = game.market?.market_id;
  const phase = game.market?.phase;
  useEffect(() => {
    if (marketId !== undefined) refreshMarket();
  }, [marketId, phase, account?.id, refreshMarket]);

  // a bootstrap that did not add up: ask the server to start again
  useEffect(() => {
    if (game.resync > 0) setSocketKey((k) => k + 1);
  }, [game.resync]);

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

      ws.onopen = () => live() && setConnected(true);
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
        setConnected(false);
        let left = RECONNECT_SECONDS;
        const show = () =>
          setStatus((s) => (s === "protocol error" ? s : `reconnecting in ${left}s`));
        show();
        retry = setInterval(() => {
          left -= 1;
          if (left > 0) return show();
          clearInterval(retry);
          connect();
        }, 1000);
      };

      function text(msg) {
        // the market half is a pure reducer: it decides what is stale, what
        // is a duplicate, and when the stream stopped making sense
        setGame((g) => reduceMarket(g, msg));

        if (msg.type === "sync_begin") {
          // the only place the renderer is reset. everything the last
          // bootstrap left is about to be sent again.
          species.current = [];
          report.current = null;
          lastSnapshot.current = null;
          runId.current = null;
          controller.current.reset();
          setDone(null);
          setFailure(null);
          setStart(null);
          setStatus("synchronising");
          return;
        }
        if (msg.type === "sync_end") {
          setStatus("watching");
          return;
        }
        if (msg.type === "account_changed") {
          // a revision and nothing else. what changed is fetched, never
          // broadcast.
          refreshAccount();
          refreshMarket();
          return;
        }
        if (fromAnotherRun(msg, runId.current)) {
          setSocketKey((k) => k + 1);
          return;
        }

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
          runId.current = msg.run_id ?? null;
          controller.current.reset();
          setDone(null);
          setStart(msg);
        }
        if (msg.type === "epoch") report.current = msg.report;
        if (msg.type === "error") throw new Error(msg.message);
        if (msg.type === "done") setDone(msg);
      }

      function binary(buffer) {
        if (species.current.length === 0) return;
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
  }, [socketKey, refreshAccount, refreshMarket]);

  async function placeBet(marketId, outcome, stake) {
    const result = await api.bet(marketId, outcome, stake);
    setAccount((prev) =>
      prev && result.account.revision < prev.revision ? prev : result.account,
    );
    setBet(result.bet);
    setGame((g) => reduceMarket(g, { type: "market_pool", market: result.market }));
  }

  const last = hud.report;
  const cards = last?.species ?? start?.species ?? [];
  // a finished run is only reported while its own market is still the current
  // one. once the next market opens, the run card leaves and betting returns.
  const finished =
    done && (game.market?.phase === "settled" || game.market?.phase === "void");

  return (
    <div className="fixed inset-0 bg-neutral-950 font-mono text-neutral-200">
      <WorldView controller={controller.current} />

      {/* the readout is not interactive, so nothing in it may sit between the
          viewer and the world */}
      <div className="pointer-events-none absolute top-4 left-4 max-w-[min(14rem,calc(100vw-9rem))] rounded border border-neutral-800/80 bg-neutral-950/70 p-3 text-xs backdrop-blur sm:top-auto sm:bottom-4 sm:max-w-[min(22rem,calc(100vw-2rem))]">
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

      <AccountPanel
        account={account}
        onChanged={(next) => {
          setAccount(next);
          setBet(null);
          // the socket authenticates once, at connect: reopening it is what
          // makes it listen for the right account
          setSocketKey((k) => k + 1);
        }}
      />

      <BetPanel
        market={game.market}
        account={account}
        bet={bet}
        synced={game.synced}
        connected={connected}
        offset={game.offset}
        onBet={placeBet}
      />

      {/* the payout phase. the betting panel hides itself while a market is
          settled, so exactly one of the two is ever on screen. */}
      {finished && (
        <RunResult
          done={done}
          market={game.market}
          bet={bet}
          seedHex={start?.seed_hex}
          status={status}
        />
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

