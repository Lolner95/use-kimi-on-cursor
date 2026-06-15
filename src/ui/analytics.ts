import type { DailyTokenUsage, GatewayStatus, TokenUsageEvent, UsageStatsSnapshot } from "../types";
import { el } from "./components";

// ─── colour tokens ───────────────────────────────────────────────────────────
const CLR = {
  prompt:  { line: "#df3e2b", fill: "#df3e2b", glow: "rgba(223,62,43,0.12)" },
  compl:   { line: "#4caf50", fill: "#4caf50", glow: "rgba(76,175,80,0.12)"  },
  total:   "#c23520",
  req:     "#e8b87a",
  bg:      "#faf7f7",
  surface: "#ffffff",
};

// ─── helpers ─────────────────────────────────────────────────────────────────
function fmt(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000)     return (n / 1_000).toFixed(n >= 10_000 ? 0 : 1) + "K";
  return n.toLocaleString();
}

function animateCount(el: HTMLElement, target: number, dur = 900): void {
  const start = performance.now();
  const tick = (now: number) => {
    const t = Math.min((now - start) / dur, 1);
    const ease = 1 - Math.pow(1 - t, 3);
    el.textContent = fmt(Math.round(target * ease));
    if (t < 1) requestAnimationFrame(tick);
    else el.textContent = fmt(target);
  };
  requestAnimationFrame(tick);
}

// Smooth monotone cubic path through points
function smoothPath(pts: { x: number; y: number }[]): string {
  if (pts.length < 2) return "";
  const n = pts.length;
  const d: number[] = [];
  const m: number[] = [];
  for (let i = 0; i < n - 1; i++) d.push((pts[i + 1].y - pts[i].y) / (pts[i + 1].x - pts[i].x));
  m[0] = d[0];
  for (let i = 1; i < n - 1; i++) {
    m[i] = d[i - 1] === 0 || d[i] === 0 || (d[i - 1] > 0) !== (d[i] > 0) ? 0 : (d[i - 1] + d[i]) / 2;
  }
  m[n - 1] = d[n - 2];

  let path = `M ${pts[0].x},${pts[0].y}`;
  for (let i = 0; i < n - 1; i++) {
    const dx = (pts[i + 1].x - pts[i].x) / 3;
    path += ` C ${pts[i].x + dx},${pts[i].y + m[i] * dx} ${pts[i + 1].x - dx},${pts[i + 1].y - m[i + 1] * dx} ${pts[i + 1].x},${pts[i + 1].y}`;
  }
  return path;
}

function buildAreaPath(pts: { x: number; y: number }[], baseY: number): string {
  if (!pts.length) return "";
  const line = smoothPath(pts);
  return `${line} L ${pts[pts.length - 1].x},${baseY} L ${pts[0].x},${baseY} Z`;
}

// ─── SVG chart ───────────────────────────────────────────────────────────────
function renderChart(days: DailyTokenUsage[]): SVGSVGElement {
  const W = 900, H = 220, PAD = { top: 16, right: 24, bottom: 36, left: 64 };
  const innerW = W - PAD.left - PAD.right;
  const innerH = H - PAD.top  - PAD.bottom;

  const maxVal = Math.max(1, ...days.map(d => d.totalTokens));
  const n = days.length;

  const xs = days.map((_, i) => PAD.left + (i / Math.max(n - 1, 1)) * innerW);
  const py = (v: number) => PAD.top + innerH - (v / maxVal) * innerH;

  const promptPts = days.map((d, i) => ({ x: xs[i], y: py(d.promptTokens) }));
  const complPts  = days.map((d, i) => ({ x: xs[i], y: py(d.completionTokens) }));

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  svg.setAttribute("class", "w-full h-full");

  const defs = document.createElementNS("http://www.w3.org/2000/svg", "defs");

  // Gradient helpers
  const makeGrad = (id: string, color: string) => {
    const g = document.createElementNS("http://www.w3.org/2000/svg", "linearGradient");
    g.id = id; g.setAttribute("x1", "0"); g.setAttribute("y1", "0");
    g.setAttribute("x2", "0"); g.setAttribute("y2", "1");
    const s1 = document.createElementNS("http://www.w3.org/2000/svg", "stop");
    s1.setAttribute("offset", "0%"); s1.setAttribute("stop-color", color); s1.setAttribute("stop-opacity", "0.45");
    const s2 = document.createElementNS("http://www.w3.org/2000/svg", "stop");
    s2.setAttribute("offset", "100%"); s2.setAttribute("stop-color", color); s2.setAttribute("stop-opacity", "0");
    g.append(s1, s2);
    return g;
  };
  defs.append(makeGrad("grad-prompt", CLR.prompt.fill), makeGrad("grad-compl", CLR.compl.fill));
  svg.append(defs);

  // Grid lines
  const baseY = PAD.top + innerH;
  [0.25, 0.5, 0.75, 1].forEach(ratio => {
    const y = PAD.top + innerH - ratio * innerH;
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", String(PAD.left)); line.setAttribute("x2", String(PAD.left + innerW));
    line.setAttribute("y1", String(y)); line.setAttribute("y2", String(y));
    line.setAttribute("stroke", "rgba(0,0,0,0.06)"); line.setAttribute("stroke-width", "1");
    svg.append(line);
    const txt = document.createElementNS("http://www.w3.org/2000/svg", "text");
    txt.setAttribute("x", String(PAD.left - 8)); txt.setAttribute("y", String(y + 4));
    txt.setAttribute("text-anchor", "end"); txt.setAttribute("fill", "#8a8a8a");
    txt.setAttribute("font-size", "10");
    txt.textContent = fmt(Math.round(maxVal * ratio));
    svg.append(txt);
  });

  // X-axis labels (show at most 7)
  const step = Math.max(1, Math.ceil(n / 7));
  days.forEach((d, i) => {
    if (i % step !== 0 && i !== n - 1) return;
    const txt = document.createElementNS("http://www.w3.org/2000/svg", "text");
    txt.setAttribute("x", String(xs[i])); txt.setAttribute("y", String(H - 6));
    txt.setAttribute("text-anchor", "middle"); txt.setAttribute("fill", "#8a8a8a");
    txt.setAttribute("font-size", "10");
    txt.textContent = d.date.slice(5); // MM-DD
    svg.append(txt);
  });

  // Area fills
  const makeArea = (pts: { x: number; y: number }[], gradId: string) => {
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", buildAreaPath(pts, baseY));
    p.setAttribute("fill", `url(#${gradId})`);
    return p;
  };
  svg.append(makeArea(promptPts, "grad-prompt"), makeArea(complPts, "grad-compl"));

  // Lines
  const makeLine = (pts: { x: number; y: number }[], color: string) => {
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", smoothPath(pts));
    p.setAttribute("fill", "none");
    p.setAttribute("stroke", color); p.setAttribute("stroke-width", "2");
    p.setAttribute("stroke-linecap", "round");
    // Animate draw
    const len = p.getTotalLength?.() ?? 2000;
    p.style.strokeDasharray = `${len}`;
    p.style.strokeDashoffset = `${len}`;
    p.style.transition = "stroke-dashoffset 1s cubic-bezier(.4,0,.2,1)";
    setTimeout(() => { p.style.strokeDashoffset = "0"; }, 50);
    return p;
  };
  svg.append(makeLine(promptPts, CLR.prompt.line), makeLine(complPts, CLR.compl.line));

  // Hover crosshair & tooltip
  const crosshair = document.createElementNS("http://www.w3.org/2000/svg", "line");
  crosshair.setAttribute("y1", String(PAD.top)); crosshair.setAttribute("y2", String(baseY));
  crosshair.setAttribute("stroke", "rgba(0,0,0,0.18)"); crosshair.setAttribute("stroke-width", "1");
  crosshair.setAttribute("stroke-dasharray", "4 4");
  crosshair.style.display = "none";
  svg.append(crosshair);

  const tip = document.createElementNS("http://www.w3.org/2000/svg", "foreignObject");
  tip.setAttribute("width", "180"); tip.setAttribute("height", "100");
  tip.style.display = "none"; tip.style.pointerEvents = "none";

  const tipDiv = document.createElement("div");
  tipDiv.style.cssText = "background:#ffffff;border:1px solid #ece6e6;border-radius:10px;padding:10px 12px;font-size:12px;color:#5a4a48;box-shadow:0 8px 24px rgba(0,0,0,0.15)";
  tip.append(tipDiv);
  svg.append(tip);

  // Invisible overlay for mouse events
  const overlay = document.createElementNS("http://www.w3.org/2000/svg", "rect");
  overlay.setAttribute("x", String(PAD.left)); overlay.setAttribute("y", String(PAD.top));
  overlay.setAttribute("width", String(innerW)); overlay.setAttribute("height", String(innerH));
  overlay.setAttribute("fill", "transparent");

  overlay.addEventListener("mousemove", (e: MouseEvent) => {
    const rect = svg.getBoundingClientRect();
    const mx = (e.clientX - rect.left) * (W / rect.width);
    const idx = Math.min(n - 1, Math.max(0, Math.round(((mx - PAD.left) / innerW) * (n - 1))));
    const d = days[idx];
    crosshair.setAttribute("x1", String(xs[idx])); crosshair.setAttribute("x2", String(xs[idx]));
    crosshair.style.display = "";
    const tipX = xs[idx] > W / 2 ? xs[idx] - 190 : xs[idx] + 12;
    tip.setAttribute("x", String(tipX)); tip.setAttribute("y", String(PAD.top + 8));
    tipDiv.innerHTML = `
      <div style="font-weight:600;color:#5a4a48;margin-bottom:6px">${d.date}</div>
      <div style="display:flex;align-items:center;gap:6px;margin-bottom:2px">
        <span style="width:8px;height:8px;border-radius:50%;background:${CLR.prompt.line};flex-shrink:0"></span>
        Input: <b style="margin-left:auto">${fmt(d.promptTokens)}</b>
      </div>
      <div style="display:flex;align-items:center;gap:6px;margin-bottom:2px">
        <span style="width:8px;height:8px;border-radius:50%;background:${CLR.compl.line};flex-shrink:0"></span>
        Output: <b style="margin-left:auto">${fmt(d.completionTokens)}</b>
      </div>
      <div style="display:flex;align-items:center;gap:6px;border-top:1px solid #ece6e6;padding-top:4px;margin-top:4px">
        <span style="font-size:10px;color:#8a8a8a">Requests</span>
        <b style="margin-left:auto;color:#5a4a48">${d.requestCount}</b>
      </div>`;
    tip.style.display = "";
  });
  overlay.addEventListener("mouseleave", () => {
    crosshair.style.display = "none"; tip.style.display = "none";
  });
  svg.append(overlay);

  return svg;
}

// ─── KPI card ────────────────────────────────────────────────────────────────
function kpiCard(
  label: string,
  _value: number,
  accent: string,
  icon: string,
): { el: HTMLElement; counter: HTMLElement } {
  const card = el("div", "relative overflow-hidden rounded-2xl p-5 flex flex-col gap-1");
  card.style.cssText = `background:${CLR.bg};border:1px solid #ece6e6`;

  // Subtle glow blob
  const blob = el("div", "absolute -top-6 -right-6 w-24 h-24 rounded-full opacity-20 pointer-events-none");
  blob.style.background = accent;
  blob.style.filter = "blur(24px)";
  card.append(blob);

  const iconEl = el("div", "text-xl mb-1", [icon]);
  const counter = el("div", "text-3xl font-bold tabular-nums tracking-tight", ["0"]);
  counter.style.color = "#5a4a48";
  const lbl = el("div", "text-xs font-medium uppercase tracking-wider mt-1");
  lbl.style.color = "#8a8a8a";
  lbl.textContent = label;
  card.append(iconEl, counter, lbl);

  return { el: card, counter };
}

// ─── Requests log ────────────────────────────────────────────────────────────
function renderRequestLog(events: TokenUsageEvent[]): HTMLElement {
  const wrap = el("div", "space-y-1");
  if (events.length === 0) {
    const empty = el("div", "text-center py-8 text-sm");
    empty.style.color = "#8a8a8a";
    empty.textContent = "No requests yet - start a conversation in Cursor.";
    return wrap;
  }

  const maxTok = Math.max(1, ...events.map(e => e.totalTokens));

  [...events].reverse().slice(0, 50).forEach(ev => {
    const row = el("div", "flex items-center gap-3 px-3 py-2.5 rounded-xl group");
    row.style.cssText = "background:#faf7f7;transition:background 0.15s";
    row.addEventListener("mouseenter", () => row.style.background = "#f3eded");
    row.addEventListener("mouseleave", () => row.style.background = "#faf7f7");

    const time = el("div", "text-xs tabular-nums shrink-0");
    time.style.color = "#8a8a8a";
    time.style.width = "56px";
    time.textContent = new Date(ev.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

    const bars = el("div", "flex-1 flex flex-col gap-0.5 min-w-0");

    const makeBar = (v: number, color: string, label: string, total: number) => {
      const row = el("div", "flex items-center gap-2");
      const lbl = el("span", "text-xs w-12 shrink-0 text-right tabular-nums");
      lbl.style.color = "#8a8a8a";
      lbl.textContent = fmt(v);
      const track = el("div", "flex-1 h-1.5 rounded-full overflow-hidden");
      track.style.background = "#ece6e6";
      const fill = el("div", "h-full rounded-full");
      fill.style.cssText = `width:${(v / total) * 100}%;background:${color};transition:width 0.6s cubic-bezier(.4,0,.2,1)`;
      const lbl2 = el("span", "text-xs shrink-0");
      lbl2.style.color = "#b3a9a7";
      lbl2.textContent = label;
      track.append(fill);
      row.append(lbl, track, lbl2);
      return row;
    };

    bars.append(
      makeBar(ev.promptTokens, CLR.prompt.line, "in", maxTok),
      makeBar(ev.completionTokens, CLR.compl.line, "out", maxTok),
    );

    const meta = el("div", "flex flex-col items-end shrink-0 gap-0.5");
    const latEl = el("div", "text-xs tabular-nums");
    latEl.style.color = "#8a8a8a";
    latEl.textContent = ev.latencyMs > 0 ? `${(ev.latencyMs / 1000).toFixed(1)}s` : "-";
    const totEl = el("div", "text-xs font-medium tabular-nums");
    totEl.style.color = "#5a4a48";
    totEl.textContent = fmt(ev.totalTokens);
    meta.append(totEl, latEl);

    row.append(time, bars, meta);
    wrap.append(row);
  });

  return wrap;
}

// ─── Main export ─────────────────────────────────────────────────────────────
export function renderAnalytics(
  usage: UsageStatsSnapshot | null,
  _status: GatewayStatus,
): HTMLElement {
  const card = el("div", "rounded-2xl overflow-hidden");
  card.style.cssText = `background:${CLR.surface};border:1px solid #ece6e6;box-shadow:0 8px 30px rgba(0,0,0,0.08)`;

  if (!usage || usage.lifetime.requestCount === 0) {
    const empty = el("div", "p-10 text-center");
    const icon = el("div", "text-5xl mb-4", ["📊"]);
    const title = el("div", "text-lg font-semibold mb-2", ["Token Analytics"]);
    title.style.color = "#5a4a48";
    const msg = el("div", "text-sm");
    msg.style.color = "#8a8a8a";
    msg.textContent = "Start chatting in Cursor with the gateway model - your analytics will appear here.";
    empty.append(icon, title, msg);
    card.append(empty);
    return card;
  }

  // Non-null snapshot, safe for closures
  const snap = usage;

  // ── Time range state ──────────────────────────────────────────────────────
  type Range = "today" | "7d" | "30d";
  let activeRange: Range = "7d";

  const getData = (): { days: DailyTokenUsage[]; events: TokenUsageEvent[] } => {
    if (activeRange === "today") return { days: [snap.today], events: snap.recentEvents };
    if (activeRange === "7d")   return { days: snap.last7Days, events: snap.recentEvents };
    return { days: snap.last30Days, events: snap.recentEvents };
  };

  const getTotals = (days: DailyTokenUsage[]) => days.reduce(
    (acc, d) => ({
      prompt: acc.prompt + d.promptTokens,
      compl:  acc.compl  + d.completionTokens,
      total:  acc.total  + d.totalTokens,
      reqs:   acc.reqs   + d.requestCount,
    }),
    { prompt: 0, compl: 0, total: 0, reqs: 0 },
  );

  // ── Header ────────────────────────────────────────────────────────────────
  const header = el("div", "flex items-center justify-between px-6 pt-5 pb-3");
  const titleRow = el("div", "");
  const titleEl = el("div", "text-lg font-bold", ["Token Analytics"]);
  titleEl.style.color = "#5a4a48";
  const subEl = el("div", "text-xs mt-0.5");
  subEl.style.color = "#8a8a8a";
  subEl.textContent = "Prompt + completion tokens proxied to Moonshot";
  titleRow.append(titleEl, subEl);

  // Tabs
  const tabs = el("div", "flex gap-1 p-1 rounded-xl");
  tabs.style.background = "#f0eaea";
  const tabDefs: { id: Range; label: string }[] = [
    { id: "today", label: "Today" },
    { id: "7d",    label: "7 Days" },
    { id: "30d",   label: "30 Days" },
  ];
  const tabEls: Map<Range, HTMLElement> = new Map();
  tabDefs.forEach(({ id, label }) => {
    const tab = el("button", "px-3 py-1.5 text-xs font-medium rounded-lg transition-all");
    tab.style.color = id === activeRange ? "#df3e2b" : "#8a8a8a";
    tab.style.background = id === activeRange ? "#ffffff" : "transparent";
    tab.style.boxShadow = id === activeRange ? "0 1px 3px rgba(0,0,0,0.1)" : "none";
    tab.textContent = label;
    tabEls.set(id, tab);
    tab.addEventListener("click", () => {
      activeRange = id;
      updateAll();
    });
    tabs.append(tab);
  });
  header.append(titleRow, tabs);
  card.append(header);

  // ── KPI cards ─────────────────────────────────────────────────────────────
  const kpiRow = el("div", "grid grid-cols-2 lg:grid-cols-4 gap-3 px-6 pb-5");
  const k1 = kpiCard("Total Tokens",  0, CLR.total,        "⚡");
  const k2 = kpiCard("Input Tokens",  0, CLR.prompt.line,  "→");
  const k3 = kpiCard("Output Tokens", 0, CLR.compl.line,   "←");
  const k4 = kpiCard("Requests",      0, CLR.req,          "↗");
  [k1, k2, k3, k4].forEach(k => kpiRow.append(k.el));
  card.append(kpiRow);

  // ── Chart area ────────────────────────────────────────────────────────────
  const chartWrap = el("div", "mx-6 mb-5 rounded-2xl overflow-hidden");
  chartWrap.style.cssText = `background:${CLR.bg};border:1px solid #ece6e6`;

  const chartPad = el("div", "pt-4 pb-2 px-2");
  // Chart SVG placeholder
  let chartSvg: SVGSVGElement | null = null;
  chartPad.style.height = "240px";
  chartWrap.append(chartPad);

  // Legend
  const legend = el("div", "flex items-center gap-5 px-4 pb-3 pt-1");
  const makeLegend = (color: string, label: string) => {
    const item = el("div", "flex items-center gap-2 text-xs");
    item.style.color = "#5a5a5a";
    const dot = el("span", "inline-block w-2.5 h-2.5 rounded-full shrink-0");
    dot.style.background = color;
    item.append(dot, document.createTextNode(label));
    return item;
  };
  legend.append(makeLegend(CLR.prompt.line, "Input tokens"), makeLegend(CLR.compl.line, "Output tokens"));
  chartWrap.append(legend);
  card.append(chartWrap);

  // ── Request log ───────────────────────────────────────────────────────────
  const logSection = el("div", "px-6 pb-6");
  const logHeader = el("div", "flex items-center justify-between mb-3");
  const logTitle = el("div", "text-sm font-semibold", ["Recent Requests"]);
  logTitle.style.color = "#5a4a48";
  logHeader.append(logTitle);
  const logColHeaders = el("div", "flex items-center gap-3 text-xs px-3 mb-1");
  logColHeaders.style.color = "#b3a9a7";
  const timeHdr = el("span", "shrink-0");
  timeHdr.style.width = "56px";
  timeHdr.textContent = "Time";
  logColHeaders.append(timeHdr, el("span", "flex-1", ["Token ratio (in / out)"]), el("span", "shrink-0", ["Total"]));
  logSection.append(logHeader, logColHeaders);
  let logWrap = renderRequestLog([]);
  logSection.append(logWrap);
  card.append(logSection);

  // ── Lifetime footer ───────────────────────────────────────────────────────
  const footer = el("div", "mx-6 mb-4 pt-3 flex flex-wrap gap-x-6 gap-y-1 text-xs border-t");
  footer.style.cssText = "border-color:#ece6e6;color:#8a8a8a";
  const lf = snap.lifetime;
  footer.append(
    el("span", "", [`Lifetime: ${fmt(lf.totalTokens)} tokens`]),
    el("span", "", [`${fmt(lf.promptTokens)} in / ${fmt(lf.completionTokens)} out`]),
    el("span", "", [`${lf.requestCount.toLocaleString()} total requests`]),
  );
  card.append(footer);

  // ── update function ───────────────────────────────────────────────────────
  function updateAll() {
    const { days, events } = getData();
    const tot = getTotals(days);

    // Update tabs
    tabEls.forEach((el, id) => {
      const active = id === activeRange;
      el.style.color = active ? "#df3e2b" : "#8a8a8a";
      el.style.background = active ? "#ffffff" : "transparent";
      el.style.boxShadow = active ? "0 1px 3px rgba(0,0,0,0.1)" : "none";
    });

    // Animate KPI counters
    animateCount(k1.counter, tot.total, 800);
    animateCount(k2.counter, tot.prompt, 800);
    animateCount(k3.counter, tot.compl, 800);
    animateCount(k4.counter, tot.reqs, 600);

    // Rebuild chart
    if (chartSvg) chartSvg.remove();
    const displayDays = activeRange === "today"
      ? [snap.today]
      : days;
    chartSvg = renderChart(displayDays.length > 1 ? displayDays : [displayDays[0] ?? snap.today]);
    chartPad.replaceChildren(chartSvg);

    // Rebuild log
    logWrap.remove();
    logWrap = renderRequestLog(events);
    logSection.append(logWrap);
  }

  // Initial render
  updateAll();

  return card;
}
