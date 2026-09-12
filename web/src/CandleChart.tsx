// SPDX-License-Identifier: Apache-2.0
//! The candlestick chart. `lightweight-charts` draws it; this component only
//! owns the fetch, the timeframe selector, and the honesty caption underneath.
//!
//! `PricePath.tsx` drew the token's price history by hand for the
//! decision-record pages, and it does not survive contact with this
//! requirement -- see its removal note in this session's commit. It plotted
//! `last_price` from outcome measurements taken roughly hourly; a trading
//! terminal needs real OHLCV at a chosen interval with a crosshair, which is
//! a different kind of chart from a different kind of data, not a bigger
//! version of the same one.

import {useEffect, useRef, useState} from "react";
import {
  CandlestickSeries,
  ColorType,
  HistogramSeries,
  createChart,
  type IChartApi,
  type ISeriesApi,
  type MouseEventParams,
  type Time,
  type UTCTimestamp,
} from "lightweight-charts";
import { CANDLE_INTERVALS, market, type Candle, type CandleInterval } from "./api";

import {formatPrice, formatStamp} from "./format";
import { useApi } from "./useApi";

/**
 * The chart's colours, as sRGB rather than as the palette's `oklch()`.
 *
 * `lightweight-charts` parses its colour strings itself and its parser predates
 * `oklch`: handed one it throws `Failed to parse color`, and because that throw
 * happens inside the chart's own render it is uncaught and blanks the entire
 * page — not just the chart. Observed 2026-09-11; the terminal rendered as an
 * empty black rectangle with the error only visible in the console.
 *
 * So these are the palette's values converted once, here, rather than read from
 * CSS custom properties at runtime. **They must be kept in step with
 * `index.css` by hand**, which is a real cost and the reason it is written down:
 * the alternative is reading the computed value and converting `oklch` to sRGB
 * in this file, which is a colour-space conversion nobody should hand-roll to
 * style a chart.
 */
const CHART_COLORS = {
  /** `--color-dim`, the axis labels. */
  dim: "#9aa0ab",
  /** `--color-line`, the grid and the scale borders. */
  line: "#3a3f47",
  /** A fainter line still, for the volume histogram's baseline. */
  faint: "#5c626b",
  /** `--color-good`, a candle that closed up. */
  up: "#5fd39a",
  /** `--color-bad`, a candle that closed down. */
  down: "#f08a5d",
  /** The same two at half opacity, for volume bars under the candles. */
  upSoft: "rgba(95, 211, 154, 0.5)",
  downSoft: "rgba(240, 138, 93, 0.5)",
} as const;

const INTERVAL_LABEL: Record<CandleInterval, string> = {
  "1m": "1m",
  "5m": "5m",
  "15m": "15m",
  "1h": "1h",
  "4h": "4h",
  "1d": "1D",
};

export function CandleChart({ mint }: { mint: string }) {
  const [interval, setInterval] = useState<CandleInterval>("15m");
  const load = useApi(
    (signal) => market.candles(mint, { interval }, signal),
    [mint, interval],
  );

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between gap-2 border-b border-[var(--color-line)] px-3 py-1.5">
        <div className="flex gap-1">
          {CANDLE_INTERVALS.map((i) => (
            <button
              key={i}
              type="button"
              onClick={() => setInterval(i)}
              aria-pressed={i === interval}
              className={`rounded px-2 py-0.5 text-xs ${
                i === interval
                  ? "bg-[var(--color-ink)] text-[var(--color-text)]"
                  : "text-[var(--color-dim)] hover:text-[var(--color-text)]"
              }`}
            >
              {INTERVAL_LABEL[i]}
            </button>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1">
        {load.state === "loading" && (
          <Placeholder text="Reading candles…" />
        )}
        {load.state === "failed" && (
          <Placeholder
            text={`Could not read the candle feed: ${load.detail}. This is not a statement about the token's price.`}
            warn
          />
        )}
        {load.state === "ready" && load.value.candles.length === 0 && (
          <Placeholder text="No candles recorded for this interval yet." />
        )}
        {load.state === "ready" && load.value.candles.length > 0 && (
          <Chart
            candles={load.value.candles}
            interval={load.value.interval}
            from={load.value.covered.from}
            to={load.value.covered.to}
            complete={load.value.covered.complete}
          />
        )}
      </div>
    </div>
  );
}

function Placeholder({ text, warn = false }: { text: string; warn?: boolean }) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-center text-sm">
      <p className={warn ? "text-[var(--color-warn)]" : "text-[var(--color-dim)]"}>{text}</p>
    </div>
  );
}

/** Renders once real candles exist, so the chart library never has to handle
 *  an empty series -- that state is `Placeholder`'s job above. */
function Chart({
  candles,
  interval,
  from,
  to,
  complete,
}: {
  candles: Candle[];
  interval: CandleInterval;
  /** The covered range, as the server's UTC stamps. Text, not epoch. */
  from: string;
  to: string;
  /** The server's own statement about whether it covered what was asked for. */
  complete: boolean;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volumeRef = useRef<ISeriesApi<"Histogram"> | null>(null);
  const [crosshair, setCrosshair] = useState<Candle | null>(null);

  // Chart lifecycle: created once per mount of a container, torn down on
  // unmount. Recreating it on every candle update would drop the reader's
  // zoom and scroll position on every refresh.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const chart = createChart(container, {
      layout: {
        background: { type: ColorType.Solid, color: "transparent" },
        textColor: CHART_COLORS.dim,
        fontSize: 11,
      },
      grid: {
        vertLines: { color: CHART_COLORS.line },
        horzLines: { color: CHART_COLORS.line },
      },
      rightPriceScale: { borderColor: CHART_COLORS.line },
      timeScale: { borderColor: CHART_COLORS.line, timeVisible: true },
      crosshair: { mode: 0 },
    });

    const series = chart.addSeries(CandlestickSeries, {
      upColor: CHART_COLORS.up,
      downColor: CHART_COLORS.down,
      borderVisible: false,
      wickUpColor: CHART_COLORS.up,
      wickDownColor: CHART_COLORS.down,
    });

    const volume = chart.addSeries(HistogramSeries, {
      color: CHART_COLORS.faint,
      priceFormat: { type: "volume" },
      priceScaleId: "",
    });
    volume.priceScale().applyOptions({ scaleMargins: { top: 0.8, bottom: 0 } });

    chartRef.current = chart;
    seriesRef.current = series;
    volumeRef.current = volume;

    chart.subscribeCrosshairMove((param: MouseEventParams<Time>) => {
      const point = param.seriesData.get(series);
      if (point && "open" in point) {
        setCrosshair({
          time: Number(param.time),
          open: point.open,
          high: point.high,
          low: point.low,
          close: point.close,
          volume: 0,
        });
      } else {
        setCrosshair(null);
      }
    });

    const resize = new ResizeObserver(() => {
      chart.applyOptions({ width: container.clientWidth, height: container.clientHeight });
    });
    resize.observe(container);

    return () => {
      resize.disconnect();
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
      volumeRef.current = null;
    };
  }, []);

  useEffect(() => {
    const series = seriesRef.current;
    const volume = volumeRef.current;
    if (!series || !volume) return;
    series.setData(
      candles.map((c) => ({
        time: c.time as UTCTimestamp,
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
      })),
    );
    volume.setData(
      candles.map((c) => ({
        time: c.time as UTCTimestamp,
        value: c.volume,
        color: c.close >= c.open ? CHART_COLORS.upSoft : CHART_COLORS.downSoft,
      })),
    );
    chartRef.current?.timeScale().fitContent();
  }, [candles]);

  const last = candles.at(-1) ?? null;
  const readout = crosshair ?? last;

  // The server says whether it covered the range asked for. It used to be
  // inferred here by comparing the returned candles' edges against the
  // window -- a guess standing in for a fact the response already carried,
  // and one that read "complete" for any window whose first and last candle
  // happened to sit at its edges.
  const narrower = !complete;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-baseline gap-3 px-3 py-1 text-xs tabular-nums text-[var(--color-dim)]">
        {readout ? (
          <>
            <span>O {formatPrice(readout.open)}</span>
            <span>H {formatPrice(readout.high)}</span>
            <span>L {formatPrice(readout.low)}</span>
            <span>C {formatPrice(readout.close)}</span>
          </>
        ) : (
          <span>&nbsp;</span>
        )}
      </div>
      <div ref={containerRef} className="min-h-0 flex-1" />
      <p className="border-t border-[var(--color-line)] px-3 py-1 text-[10px] text-[var(--color-dim)]">
        {interval} candles, {formatStamp(from)} – {formatStamp(to)}
        {narrower ? " — narrower than the requested range; this is what Radar has, not the whole history." : ""}
      </p>
    </div>
  );
}
