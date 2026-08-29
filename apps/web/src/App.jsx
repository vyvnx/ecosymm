import { useCallback, useEffect, useRef, useState } from "react";
import AccountPanel from "./AccountPanel.jsx";
import BetPanel from "./BetPanel.jsx";
import BettingStage from "./BettingStage.jsx";
import LiveLog from "./LiveLog.jsx";
import RunResult from "./RunResult.jsx";
import SpeciesProfiles from "./SpeciesProfiles.jsx";
import SpectatorDock from "./SpectatorDock.jsx";
import WorldView, { createController } from "./WorldView.jsx";
import { api } from "./game/api.js";
import { fromAnotherRun, initialMarket, reduceMarket } from "./game/market.js";
import { decode } from "./render/protocol.js";
import { speciesCss } from "./render/WorldRenderer.js";
import { initialFeed, markRead, reduceFeed, resetFeed, unread } from "./telemetry/events.js";
import { profile } from "./telemetry/species.js";

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
  const [form, setForm] = useState([]);
  const [feed, setFeed] = useState(initialFeed);
  // the one place that decides what may be on screen at once. individual
  // panels ask for a slot; none of them decides it may have one.
  const [panel, setPanel] = useState(null);
  const [rails, setRails] = useState({ species: true, events: true });
  // bumped whenever the socket has to start again: a bootstrap that did not
  // add up, or signing in and out, which is what re-authenticates it
  const [socketKey, setSocketKey] = useState(0);

  const report = useRef(null);
  const species = useRef([]);
  // the run's first reported epoch, kept as the founder baseline every meter
  // is drawn against. one report, replaced only by a new run.
  const founder = useRef(null);
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

  // the viewer reached the bottom of the feed, so what is held is now read
  const seen = useCallback(() => setFeed(markRead), []);

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

  // a panel opened while the run was live has no business surviving into the
  // next phase, so the machine starts closed every time the phase moves
  useEffect(() => {
    setPanel(null);
  }, [phase]);

  // the record the betting phase reads back. only a market finishing changes
  // it, so a new market id is the whole of when to ask for it again.
  useEffect(() => {
    api.form().then(setForm).catch(() => {});
  }, [marketId]);

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
          founder.current = null;
          lastSnapshot.current = null;
          runId.current = null;
          setFeed(initialFeed);
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
          founder.current = null;
          lastSnapshot.current = null;
          runId.current = msg.run_id ?? null;
          setFeed(resetFeed(msg.run_id ?? null));
          controller.current.reset();
          setDone(null);
          setStart(msg);
        }
        if (msg.type === "epoch") {
          report.current = msg.report;
          founder.current ??= msg.report.species;
        }
        if (msg.type === "telemetry") setFeed((f) => reduceFeed(f, msg));
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

  // the overlay state machine. one expression decides everything that may be
  // on screen at once, so no panel has to guess whether it is allowed to open:
  // a finished run outranks every panel, and an open market outranks
  // telemetry, because the run behind the dim is not the one being bet on.
  const betting = phase === "open";
  const overlay = finished ? "result" : betting ? null : panel;
  const sheet = overlay === "events" || overlay === "species" ? overlay : null;
  const watching = !betting && !finished;

  // ponytail: rebuilt on the hud's 100ms tick for two species. it is a handful
  // of divisions - memoise it if the scenario ever grows a lot more species.
  const profiles =
    watching && last
      ? last.species.map((s, i) =>
          profile(s, founder.current?.[i] ?? null, start?.gene_bounds, {
            index: i,
            events: feed.events,
            epoch: last.epoch,
          }),
        )
      : [];

  return (
    <div className="fixed inset-0 bg-neutral-950 font-mono text-neutral-200">
      <WorldView controller={controller.current} />

      {/* one compact strip on a phone: the epoch and who is alive, and nothing
          else. the full readout is a desktop luxury - on a 320px screen every
          line of it is a line the world does not get. */}
      <div className="pointer-events-none absolute top-4 right-[7.5rem] left-4 flex items-baseline gap-2 overflow-hidden rounded border border-neutral-800/80 bg-neutral-950/70 px-2 py-1 text-xs whitespace-nowrap backdrop-blur sm:hidden">
        <span className="font-bold text-emerald-400">ecosym</span>
        {last ? (
          <>
            <span className="tabular-nums text-neutral-400">e{last.epoch.toLocaleString()}</span>
            {cards.map((s, i) => (
              <span key={s.id} className="flex items-baseline gap-1">
                <span
                  aria-hidden
                  className="h-1.5 w-1.5 rounded-full"
                  style={{ background: speciesCss(i) }}
                />
                <span className="sr-only">{s.name} </span>
                <span className="tabular-nums text-neutral-200">
                  {(s.population ?? 0).toLocaleString()}
                </span>
              </span>
            ))}
          </>
        ) : (
          <span className="truncate text-neutral-400">{status}</span>
        )}
        {failure && <span className="truncate text-amber-400">{failure}</span>}
      </div>

      {/* the readout is not interactive, so nothing in it may sit between the
          viewer and the world */}
      <div className="pointer-events-none absolute top-4 left-4 hidden max-w-[min(14rem,calc(100vw-9rem))] rounded border border-neutral-800/80 bg-neutral-950/70 p-3 text-xs backdrop-blur sm:top-auto sm:bottom-4 sm:block sm:max-w-[min(22rem,calc(100vw-2rem))]">
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

      {/* the betting phase. the map behind it belongs to the run that just
          ended, so it goes dark and the record takes the screen instead. */}
      <BettingStage market={game.market} form={form} />

      {/* the two desktop rails. they collapse independently and the world
          stays where it is: it is centred by the canvas itself, so nothing
          here can squash its aspect ratio. */}
      {watching && (
        <>
          <Rail
            side="left"
            title="species"
            open={rails.species}
            onToggle={() => setRails((r) => ({ ...r, species: !r.species }))}
          >
            <SpeciesProfiles cards={profiles} />
          </Rail>
          <Rail
            side="right"
            title="live"
            badge={unread(feed).length}
            open={rails.events}
            onToggle={() => setRails((r) => ({ ...r, events: !r.events }))}
          >
            <LiveLog feed={feed} onSeen={seen} label="run events" />
          </Rail>

          <SpectatorDock
            feed={feed}
            cards={profiles}
            sheet={sheet}
            onOpen={setPanel}
            onClose={() => setPanel(null)}
            onSeen={seen}
          />
        </>
      )}

      <AccountPanel
        account={account}
        open={overlay === "account"}
        onToggle={() =>
          setPanel((p) => (p === "account" ? null : "account"))
        }
        onChanged={(next) => {
          setAccount(next);
          setBet(null);
          setPanel(null);
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
        expanded={betting}
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

/**
 * one desktop rail. it is a panel over the world rather than a column beside
 * it, because the canvas is full-bleed and letterboxes the map itself - a
 * layout column would take width from the world for nothing.
 */
function Rail({ side, title, open, onToggle, badge = 0, children }) {
  // anchored top and bottom rather than capped by height: a short landscape
  // window is exactly where a max-height rail grows down into the readout
  const place =
    side === "left"
      ? "left-4 top-4 bottom-[12rem] w-52 lg:w-60"
      : "right-4 top-16 bottom-[6rem] w-60 lg:w-72";
  return (
    <div
      className={`pointer-events-auto absolute ${open ? place : `${place} bottom-auto`} hidden flex-col rounded border border-neutral-800/80 bg-neutral-950/70 p-2 text-xs backdrop-blur sm:flex`}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex min-h-6 items-baseline gap-2 text-neutral-500 hover:text-neutral-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
      >
        <span>{title}</span>
        {badge > 0 && !open && (
          <span className="rounded-full bg-emerald-950/60 px-1.5 tabular-nums text-emerald-300">
            {badge}
          </span>
        )}
        <span aria-hidden className="ml-auto text-neutral-700">
          {open ? "\u2013" : "+"}
        </span>
      </button>
      {open && (
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden pt-2">
          {children}
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

