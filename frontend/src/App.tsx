/**
 * tokenscale dashboard — root component.
 *
 * Phase 1 surface:
 *   - One chart: daily token usage stacked by model.
 *   - One filter: provider selector. v1 has only `anthropic`, but the control
 *     ships from day one so the UI does not change in v2.
 *   - A date-range pair (last 30 days by default).
 *
 * Data source is the Rust API on the same origin: `/api/v1/usage/daily`. In
 * dev, Vite's proxy forwards to 127.0.0.1:8787; in production, the Rust binary
 * serves both the API and the embedded SPA.
 */

import { useEffect, useMemo, useState } from 'react'
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

type ProviderFilter = 'all' | 'anthropic'

/** Per-token-type breakdown for one (date, model). Mirrors the server's
 *  `ModelTokens` shape. `billable_total` is absent when pricing.toml has no
 *  entry for the model — the dashboard falls back to raw counts in that case.
 */
type ModelTokens = {
  input: number
  output: number
  cache_read: number
  cache_write_5m: number
  cache_write_1h: number
  billable_total?: number
}

type DailyUsageRow = {
  /** ISO-8601 date — `YYYY-MM-DD`. */
  date: string
  /** Per-model breakdown for that day. Keys are model identifiers. */
  byModel: Record<string, ModelTokens>
}

type DailyUsageResponse = {
  rows: DailyUsageRow[]
  /** Models that appear anywhere in `rows`, in chart-order. */
  models: string[]
  /** Stable order for per-token-type controls (Iteration B). */
  tokenTypes: string[]
  /** Models that appeared in the data but had no pricing entry. */
  modelsWithoutPricing: string[]
}

/** Sum the five token-type fields. Used as the chart value while Iteration A
 *  keeps the existing "all tokens" view; Iteration B will replace this with
 *  filter-aware aggregation.
 */
function totalTokensForModel(tokens: ModelTokens): number {
  return tokens.input + tokens.output + tokens.cache_read + tokens.cache_write_5m + tokens.cache_write_1h
}

type FetchState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ok'; data: DailyUsageResponse }
  | { status: 'error'; message: string }

const DEFAULT_WINDOW_DAYS = 30

/** Returns ISO `YYYY-MM-DD` for `daysAgo` days before today (UTC). */
function isoDateDaysAgo(daysAgo: number): string {
  const now = new Date()
  now.setUTCDate(now.getUTCDate() - daysAgo)
  return now.toISOString().slice(0, 10)
}

export default function App() {
  const [providerFilter, setProviderFilter] = useState<ProviderFilter>('all')
  const [fromDate] = useState<string>(() => isoDateDaysAgo(DEFAULT_WINDOW_DAYS))
  const [toDate] = useState<string>(() => isoDateDaysAgo(0))
  const [fetchState, setFetchState] = useState<FetchState>({ status: 'idle' })

  useEffect(() => {
    const abortController = new AbortController()

    async function loadDailyUsage() {
      setFetchState({ status: 'loading' })
      try {
        const queryString = new URLSearchParams({
          from: fromDate,
          to: toDate,
          provider: providerFilter,
        }).toString()
        const response = await fetch(`/api/v1/usage/daily?${queryString}`, {
          signal: abortController.signal,
        })
        if (!response.ok) {
          throw new Error(`HTTP ${response.status}`)
        }
        const data = (await response.json()) as DailyUsageResponse
        setFetchState({ status: 'ok', data })
      } catch (error) {
        if ((error as Error).name === 'AbortError') return
        setFetchState({
          status: 'error',
          message: (error as Error).message,
        })
      }
    }

    loadDailyUsage()
    return () => abortController.abort()
  }, [providerFilter, fromDate, toDate])

  // Zero-fill missing (date, model) combos. Without this, a model that
  // wasn't used on a given day shows up as undefined to Recharts, which
  // renders a visual cliff between adjacent days. With it, areas ramp
  // smoothly from zero — accurate when (e.g.) a new model release replaces
  // an older one mid-window.
  //
  // Iteration A still shows raw "all token types summed" per model. Iteration
  // B will branch this on the upcoming view-mode and token-type filter
  // controls.
  const chartRows = useMemo(() => {
    if (fetchState.status !== 'ok') return []
    const allModels = fetchState.data.models
    return fetchState.data.rows.map((row) => {
      const filled: Record<string, string | number> = { date: row.date }
      for (const modelIdentifier of allModels) {
        const modelTokens = row.byModel[modelIdentifier]
        filled[modelIdentifier] = modelTokens ? totalTokensForModel(modelTokens) : 0
      }
      return filled
    })
  }, [fetchState])

  const chartModels = fetchState.status === 'ok' ? fetchState.data.models : []

  return (
    <div className="min-h-screen bg-slate-50 text-slate-900">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto max-w-6xl px-6 py-4 flex items-center justify-between">
          <h1 className="text-xl font-semibold tracking-tight">tokenscale</h1>
          <span className="text-xs text-slate-500">Phase 1 — local Claude Code only</span>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-6 py-8 space-y-6">
        <section className="bg-white rounded-lg border border-slate-200 p-5">
          <div className="flex flex-wrap items-end gap-4 mb-4">
            <div>
              <label className="block text-xs font-medium text-slate-600 mb-1" htmlFor="provider-filter">
                Provider
              </label>
              <select
                id="provider-filter"
                className="border border-slate-300 rounded-md px-3 py-1.5 text-sm bg-white"
                value={providerFilter}
                onChange={(event) => setProviderFilter(event.target.value as ProviderFilter)}
              >
                <option value="all">All providers</option>
                <option value="anthropic">Anthropic</option>
              </select>
            </div>
            <div className="text-xs text-slate-500">
              {fromDate} &rarr; {toDate}
            </div>
          </div>

          <h2 className="text-base font-medium mb-3">Daily token usage by model</h2>

          <div className="h-80">
            {fetchState.status === 'loading' && (
              <div className="flex h-full items-center justify-center text-sm text-slate-500">Loading…</div>
            )}
            {fetchState.status === 'error' && (
              <div className="flex h-full flex-col items-center justify-center text-sm text-slate-500">
                <p>Could not reach the tokenscale API.</p>
                <p className="text-xs mt-1">{fetchState.message}</p>
                <p className="text-xs mt-3">
                  Run <code className="bg-slate-100 px-1 rounded">tokenscale serve</code> in another shell.
                </p>
              </div>
            )}
            {fetchState.status === 'ok' && chartRows.length === 0 && (
              <div className="flex h-full items-center justify-center text-sm text-slate-500">
                No usage in the selected window.
              </div>
            )}
            {fetchState.status === 'ok' && chartRows.length > 0 && (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={chartRows} margin={{ top: 8, right: 16, left: 8, bottom: 0 }}>
                  <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                  <XAxis dataKey="date" tick={{ fontSize: 12 }} />
                  <YAxis
                    tick={{ fontSize: 12 }}
                    tickFormatter={formatCompactNumber}
                    width={56}
                  />
                  <Tooltip
                    formatter={(rawValue, modelDisplayLabel) => [
                      typeof rawValue === 'number' ? rawValue.toLocaleString() : String(rawValue),
                      modelDisplayLabel,
                    ]}
                  />
                  <Legend />
                  {chartModels.map((modelIdentifier, modelIndex) => (
                    <Area
                      key={modelIdentifier}
                      type="monotone"
                      dataKey={modelIdentifier}
                      name={modelDisplayName(modelIdentifier)}
                      stackId="1"
                      stroke={CHART_COLORS[modelIndex % CHART_COLORS.length]}
                      fill={CHART_COLORS[modelIndex % CHART_COLORS.length]}
                      fillOpacity={0.6}
                    />
                  ))}
                </AreaChart>
              </ResponsiveContainer>
            )}
          </div>
        </section>
      </main>
    </div>
  )
}

/**
 * Chart palette. Cycled through model series in stable order so re-renders
 * with the same model set produce the same colors.
 */
const CHART_COLORS = ['#2563eb', '#16a34a', '#d97706', '#9333ea', '#dc2626', '#0891b2']

/**
 * Compact human-readable formatter for the y-axis. The dashboard's token
 * counts run from thousands to billions, and rendering raw integers
 * eats more horizontal space than the chart can spare.
 */
function formatCompactNumber(value: number): string {
  const absoluteValue = Math.abs(value)
  if (absoluteValue >= 1e9) return `${stripTrailingZero((value / 1e9).toFixed(1))}B`
  if (absoluteValue >= 1e6) return `${stripTrailingZero((value / 1e6).toFixed(1))}M`
  if (absoluteValue >= 1e3) return `${stripTrailingZero((value / 1e3).toFixed(1))}K`
  return value.toString()
}

function stripTrailingZero(formatted: string): string {
  return formatted.endsWith('.0') ? formatted.slice(0, -2) : formatted
}

/**
 * Convert an Anthropic-style model identifier (`claude-opus-4-7`) into the
 * marketing label users actually recognize (`Claude Opus 4.7`). Unknown
 * identifiers pass through unchanged so v2 providers don't silently lose
 * their labels — better to show the raw id than to invent a name.
 */
function modelDisplayName(modelIdentifier: string): string {
  const claudeFamilyMatch = modelIdentifier.match(
    /^claude-(opus|sonnet|haiku)-(\d+)-(\d+)$/,
  )
  if (claudeFamilyMatch) {
    const [, modelFamily, majorVersion, minorVersion] = claudeFamilyMatch
    const familyTitle = modelFamily.charAt(0).toUpperCase() + modelFamily.slice(1)
    return `Claude ${familyTitle} ${majorVersion}.${minorVersion}`
  }
  return modelIdentifier
}
