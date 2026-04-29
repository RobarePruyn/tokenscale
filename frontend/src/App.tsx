/**
 * tokenscale dashboard — root component.
 *
 * Phase 1 surface (Iteration C1):
 *   - Header subtitle, "Understand your impact."
 *   - Date range — presets (7d / 30d / 90d / 1y / All) + custom from/to.
 *   - Collapsible filter section with a one-line summary on the collapsed row.
 *   - Multi-select chips (models / token types / projects) with per-group
 *     "Select all" + "Select none" buttons.
 *   - Stack-by (Model | Token type) and Tokens (All raw | Billable) radios.
 *   - Pricing-needs-review banner above the chart when applicable.
 *
 * Data sources:
 *   - GET /api/v1/health           — pricing-file status banner
 *   - GET /api/v1/projects         — populate the project chip list
 *   - GET /api/v1/usage/daily      — chart data, refetched when filters
 *                                    that affect SQL change (provider,
 *                                    projects, dates)
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

// ---------------------------------------------------------------------------
// API types — mirror the Rust `Json` shapes
// ---------------------------------------------------------------------------

type ProviderFilter = 'all' | 'anthropic'

type BillableBreakdown = {
  input: number
  output: number
  cache_read: number
  cache_write_5m: number
  cache_write_1h: number
}

type ModelTokens = {
  input: number
  output: number
  cache_read: number
  cache_write_5m: number
  cache_write_1h: number
  billable?: BillableBreakdown
  billable_total?: number
}

type DailyUsageRow = {
  date: string
  byModel: Record<string, ModelTokens>
}

type DailyUsageResponse = {
  rows: DailyUsageRow[]
  models: string[]
  tokenTypes: string[]
  modelsWithoutPricing: string[]
}

type ProjectsResponse = {
  projects: Array<{
    project_id: string
    event_count: number
    total_tokens: number
  }>
}

type HealthResponse = {
  status: string
  version: string
  total_events: number
  providers: string[]
  pricing: {
    schema_version: number
    file_status: string
    model_count: number
    needs_review: boolean
  }
}

// ---------------------------------------------------------------------------
// View-state types
// ---------------------------------------------------------------------------

type StackBy = 'model' | 'token-type'
type ViewMode = 'all' | 'billable'
type RangePreset = '7d' | '30d' | '90d' | '365d' | 'all' | 'custom'

/** A `null` selection means "everything selected" — preserves the all-on
 *  default without forcing us to seed the Set from data we haven't loaded
 *  yet. An empty `Set` means the user explicitly selected nothing.
 */
type Selection = Set<string> | null

type FetchState<T> =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ok'; data: T }
  | { status: 'error'; message: string }

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_RANGE_PRESET: RangePreset = '30d'

/** Sentinel `?project=` value mapped on the server to "match no projects" —
 *  used when the user clicks "Select none" in the projects chip group.
 */
const PROJECTS_NONE_SENTINEL = '__none__'

/** Lower bound for the "All time" preset. The server clamps to whatever's
 *  in the database, so picking a date well before any conceivable Claude
 *  Code session is the simplest "no lower bound" sentinel.
 */
const ALL_TIME_FROM_DATE = '2000-01-01'

const TOKEN_TYPE_DISPLAY_NAMES: Record<string, string> = {
  input: 'Input',
  output: 'Output',
  cache_read: 'Cache read',
  cache_write_5m: 'Cache write (5m)',
  cache_write_1h: 'Cache write (1h)',
}

const RANGE_PRESET_LABELS: Array<{ value: RangePreset; label: string }> = [
  { value: '7d', label: '7d' },
  { value: '30d', label: '30d' },
  { value: '90d', label: '90d' },
  { value: '365d', label: '1y' },
  { value: 'all', label: 'All' },
  { value: 'custom', label: 'Custom' },
]

const CHART_COLORS = [
  '#2563eb',
  '#16a34a',
  '#d97706',
  '#9333ea',
  '#dc2626',
  '#0891b2',
  '#db2777',
  '#65a30d',
]

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

function isoDateDaysAgo(daysAgo: number): string {
  const now = new Date()
  now.setUTCDate(now.getUTCDate() - daysAgo)
  return now.toISOString().slice(0, 10)
}

function effectiveDateRange(
  preset: RangePreset,
  customFrom: string,
  customTo: string,
): { fromDate: string; toDate: string } {
  const today = isoDateDaysAgo(0)
  switch (preset) {
    case '7d':
      return { fromDate: isoDateDaysAgo(7), toDate: today }
    case '30d':
      return { fromDate: isoDateDaysAgo(30), toDate: today }
    case '90d':
      return { fromDate: isoDateDaysAgo(90), toDate: today }
    case '365d':
      return { fromDate: isoDateDaysAgo(365), toDate: today }
    case 'all':
      return { fromDate: ALL_TIME_FROM_DATE, toDate: today }
    case 'custom':
      return { fromDate: customFrom, toDate: customTo }
  }
}

function isSelected(selection: Selection, value: string): boolean {
  return selection === null || selection.has(value)
}

/** Toggle a value, preserving "all by default" semantics. */
function toggleSelection(
  selection: Selection,
  value: string,
  knownValues: string[],
): Selection {
  if (selection === null) {
    return new Set(knownValues.filter((known) => known !== value))
  }
  const next = new Set(selection)
  if (next.has(value)) next.delete(value)
  else next.add(value)
  return next.size === knownValues.length ? null : next
}

function selectionSummary(
  pluralLabel: string,
  selection: Selection,
  total: number,
): string {
  if (total === 0) return `no ${pluralLabel}`
  if (selection === null) return `all ${total} ${pluralLabel}`
  if (selection.size === 0) return `0 of ${total} ${pluralLabel}`
  return `${selection.size} of ${total} ${pluralLabel}`
}

/** Map Anthropic's machine identifier (`claude-opus-4-7`) to the marketing
 *  label (`Claude Opus 4.7`). Unknown identifiers pass through unchanged.
 */
function modelDisplayName(modelIdentifier: string): string {
  const claudeFamilyMatch = modelIdentifier.match(
    /^claude-(opus|sonnet|haiku)-(\d+)-(\d+)$/,
  )
  if (claudeFamilyMatch) {
    const [, family, major, minor] = claudeFamilyMatch
    return `Claude ${family.charAt(0).toUpperCase() + family.slice(1)} ${major}.${minor}`
  }
  return modelIdentifier
}

function tokenTypeDisplayName(tokenType: string): string {
  return TOKEN_TYPE_DISPLAY_NAMES[tokenType] ?? tokenType
}

function projectShortName(projectPath: string): string {
  const parts = projectPath.split('/').filter(Boolean)
  const last = parts[parts.length - 1] ?? projectPath
  return last.length > 40 ? `${last.slice(0, 37)}…` : last
}

function formatCompactNumber(value: number): string {
  const absolute = Math.abs(value)
  if (absolute >= 1e9) return `${stripTrailingZero((value / 1e9).toFixed(1))}B`
  if (absolute >= 1e6) return `${stripTrailingZero((value / 1e6).toFixed(1))}M`
  if (absolute >= 1e3) return `${stripTrailingZero((value / 1e3).toFixed(1))}K`
  return value.toString()
}

function stripTrailingZero(formatted: string): string {
  return formatted.endsWith('.0') ? formatted.slice(0, -2) : formatted
}

function tokenFieldsForView(
  modelTokens: ModelTokens,
  viewMode: ViewMode,
): BillableBreakdown {
  if (viewMode === 'billable' && modelTokens.billable) {
    return modelTokens.billable
  }
  return {
    input: modelTokens.input,
    output: modelTokens.output,
    cache_read: modelTokens.cache_read,
    cache_write_5m: modelTokens.cache_write_5m,
    cache_write_1h: modelTokens.cache_write_1h,
  }
}

function sumSelectedTokenFields(
  fields: BillableBreakdown,
  visibleTokenTypes: Set<string>,
): number {
  let total = 0
  if (visibleTokenTypes.has('input')) total += fields.input
  if (visibleTokenTypes.has('output')) total += fields.output
  if (visibleTokenTypes.has('cache_read')) total += fields.cache_read
  if (visibleTokenTypes.has('cache_write_5m')) total += fields.cache_write_5m
  if (visibleTokenTypes.has('cache_write_1h')) total += fields.cache_write_1h
  return total
}

/** Encode a project Selection as the value of the `?project=` query param,
 *  or `null` to omit the param entirely (server default = "no filter").
 */
function encodeProjectParam(selection: Selection): string | null {
  if (selection === null) return null // implicit all → omit param
  if (selection.size === 0) return PROJECTS_NONE_SENTINEL // explicit none
  return Array.from(selection).join(',')
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export default function App() {
  const [providerFilter, setProviderFilter] = useState<ProviderFilter>('all')

  const [rangePreset, setRangePreset] = useState<RangePreset>(DEFAULT_RANGE_PRESET)
  const [customFromDate, setCustomFromDate] = useState<string>(() => isoDateDaysAgo(30))
  const [customToDate, setCustomToDate] = useState<string>(() => isoDateDaysAgo(0))
  const { fromDate, toDate } = effectiveDateRange(rangePreset, customFromDate, customToDate)

  const [selectedModels, setSelectedModels] = useState<Selection>(null)
  const [selectedTokenTypes, setSelectedTokenTypes] = useState<Selection>(null)
  const [selectedProjects, setSelectedProjects] = useState<Selection>(null)

  const [filtersExpanded, setFiltersExpanded] = useState<boolean>(false)

  const [stackBy, setStackBy] = useState<StackBy>('model')
  const [viewMode, setViewMode] = useState<ViewMode>('all')

  const [healthState, setHealthState] = useState<FetchState<HealthResponse>>({ status: 'idle' })
  const [projectsState, setProjectsState] = useState<FetchState<ProjectsResponse>>({
    status: 'idle',
  })
  const [dailyState, setDailyState] = useState<FetchState<DailyUsageResponse>>({ status: 'idle' })

  // Health — once on mount.
  useEffect(() => {
    let cancelled = false
    fetch('/api/v1/health')
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as HealthResponse
      })
      .then((data) => {
        if (!cancelled) setHealthState({ status: 'ok', data })
      })
      .catch((error) => {
        if (!cancelled) setHealthState({ status: 'error', message: (error as Error).message })
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Projects — when window/provider changes (chip list itself doesn't depend
  // on the user's selection).
  useEffect(() => {
    const abort = new AbortController()
    setProjectsState({ status: 'loading' })
    const params = new URLSearchParams({ from: fromDate, to: toDate, provider: providerFilter })
    fetch(`/api/v1/projects?${params.toString()}`, { signal: abort.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as ProjectsResponse
      })
      .then((data) => setProjectsState({ status: 'ok', data }))
      .catch((error) => {
        if ((error as Error).name === 'AbortError') return
        setProjectsState({ status: 'error', message: (error as Error).message })
      })
    return () => abort.abort()
  }, [providerFilter, fromDate, toDate])

  // Daily — refetched when SQL filters change.
  useEffect(() => {
    const abort = new AbortController()
    setDailyState({ status: 'loading' })
    const params = new URLSearchParams({ from: fromDate, to: toDate, provider: providerFilter })
    const projectParam = encodeProjectParam(selectedProjects)
    if (projectParam !== null) params.set('project', projectParam)
    fetch(`/api/v1/usage/daily?${params.toString()}`, { signal: abort.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as DailyUsageResponse
      })
      .then((data) => setDailyState({ status: 'ok', data }))
      .catch((error) => {
        if ((error as Error).name === 'AbortError') return
        setDailyState({ status: 'error', message: (error as Error).message })
      })
    return () => abort.abort()
  }, [providerFilter, selectedProjects, fromDate, toDate])

  // ----- Derived chart config ---------------------------------------------
  const chartConfig = useMemo(() => {
    if (dailyState.status !== 'ok') return { rows: [], series: [], hiddenInBillable: [] as string[] }
    const data = dailyState.data
    const visibleTokenTypes = new Set(
      data.tokenTypes.filter((t) => isSelected(selectedTokenTypes, t)),
    )
    let visibleModels = data.models.filter((m) => isSelected(selectedModels, m))
    const hiddenInBillable: string[] = []
    if (viewMode === 'billable') {
      visibleModels = visibleModels.filter((m) => {
        if (data.modelsWithoutPricing.includes(m)) {
          hiddenInBillable.push(m)
          return false
        }
        return true
      })
    }

    if (stackBy === 'model') {
      const series = visibleModels.map((modelId, index) => ({
        key: modelId,
        displayName: modelDisplayName(modelId),
        color: CHART_COLORS[index % CHART_COLORS.length],
      }))
      const rows = data.rows.map((row) => {
        const cells: Record<string, string | number> = { date: row.date }
        for (const modelId of visibleModels) {
          const tokens = row.byModel[modelId]
          cells[modelId] = tokens
            ? sumSelectedTokenFields(tokenFieldsForView(tokens, viewMode), visibleTokenTypes)
            : 0
        }
        return cells
      })
      return { rows, series, hiddenInBillable }
    }

    // stackBy === 'token-type'
    const tokenTypeKeys = data.tokenTypes.filter((t) => visibleTokenTypes.has(t))
    const series = tokenTypeKeys.map((tokenType, index) => ({
      key: tokenType,
      displayName: tokenTypeDisplayName(tokenType),
      color: CHART_COLORS[index % CHART_COLORS.length],
    }))
    const rows = data.rows.map((row) => {
      const cells: Record<string, string | number> = { date: row.date }
      for (const tokenType of tokenTypeKeys) {
        let sum = 0
        for (const modelId of visibleModels) {
          const tokens = row.byModel[modelId]
          if (!tokens) continue
          sum += tokenFieldsForView(tokens, viewMode)[tokenType as keyof BillableBreakdown]
        }
        cells[tokenType] = sum
      }
      return cells
    })
    return { rows, series, hiddenInBillable }
  }, [dailyState, selectedModels, selectedTokenTypes, stackBy, viewMode])

  const allModels = dailyState.status === 'ok' ? dailyState.data.models : []
  const allTokenTypes = dailyState.status === 'ok' ? dailyState.data.tokenTypes : []
  const allProjectIds = projectsState.status === 'ok'
    ? projectsState.data.projects.map((p) => p.project_id)
    : []
  const pricingNeedsReview =
    healthState.status === 'ok' && healthState.data.pricing.needs_review

  const filtersHaveSelection =
    selectedModels !== null || selectedTokenTypes !== null || selectedProjects !== null
  const filterSummaryText = [
    selectionSummary('models', selectedModels, allModels.length),
    selectionSummary('token types', selectedTokenTypes, allTokenTypes.length),
    selectionSummary('projects', selectedProjects, allProjectIds.length),
  ].join(' · ')

  function chooseRangePreset(next: RangePreset) {
    if (next === 'custom' && rangePreset !== 'custom') {
      // Seed custom inputs with whatever the user is currently looking at
      // so jumping to Custom doesn't snap to defaults.
      setCustomFromDate(fromDate)
      setCustomToDate(toDate)
    }
    setRangePreset(next)
  }

  return (
    <div className="min-h-screen bg-slate-50 text-slate-900">
      <header className="border-b border-slate-200 bg-white">
        <div className="mx-auto max-w-6xl px-6 py-4 flex items-baseline justify-between gap-4">
          <div className="flex items-baseline gap-3">
            <h1 className="text-xl font-semibold tracking-tight">tokenscale</h1>
            <span className="text-sm text-slate-500">Understand your impact.</span>
          </div>
          <span className="text-xs text-slate-500">Phase 1 — local Claude Code only</span>
        </div>
      </header>

      {pricingNeedsReview && viewMode === 'billable' && (
        <div className="bg-amber-50 border-b border-amber-200 text-amber-900 text-xs px-6 py-2">
          <span className="font-medium">Pricing needs review:</span> the billable view is using
          seed values from <code className="bg-amber-100 px-1 rounded">pricing.toml</code>. Verify
          against{' '}
          <a
            className="underline"
            href="https://platform.claude.com/docs/en/about-claude/pricing"
            target="_blank"
            rel="noreferrer"
          >
            current Anthropic prices
          </a>{' '}
          before relying on these numbers.
        </div>
      )}

      <main className="mx-auto max-w-6xl px-6 py-8 space-y-6">
        <section className="bg-white rounded-lg border border-slate-200 p-5 space-y-5">
          {/* Top row: provider + window summary */}
          <div className="flex flex-wrap items-end gap-4">
            <div>
              <label
                className="block text-xs font-medium text-slate-600 mb-1"
                htmlFor="provider-filter"
              >
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
            <div className="text-xs text-slate-500 ml-auto">
              {fromDate} → {toDate}
            </div>
          </div>

          {/* Date range pills + custom inputs */}
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium text-slate-600 mr-1">Range:</span>
            <div className="inline-flex rounded-md border border-slate-300 overflow-hidden">
              {RANGE_PRESET_LABELS.map((option) => {
                const active = option.value === rangePreset
                return (
                  <button
                    key={option.value}
                    type="button"
                    onClick={() => chooseRangePreset(option.value)}
                    className={
                      'px-3 py-1 text-xs transition-colors border-l border-slate-200 first:border-l-0 ' +
                      (active
                        ? 'bg-blue-50 text-blue-700'
                        : 'bg-white text-slate-500 hover:bg-slate-50')
                    }
                  >
                    {option.label}
                  </button>
                )
              })}
            </div>
            {rangePreset === 'custom' && (
              <div className="flex items-center gap-2 text-xs text-slate-600">
                <input
                  type="date"
                  value={customFromDate}
                  max={customToDate}
                  onChange={(event) => setCustomFromDate(event.target.value)}
                  className="border border-slate-300 rounded-md px-2 py-1 bg-white"
                />
                <span>→</span>
                <input
                  type="date"
                  value={customToDate}
                  min={customFromDate}
                  onChange={(event) => setCustomToDate(event.target.value)}
                  className="border border-slate-300 rounded-md px-2 py-1 bg-white"
                />
              </div>
            )}
          </div>

          {/* Filters — collapsible */}
          <div>
            <button
              type="button"
              className="w-full flex items-center justify-between text-left border border-slate-200 hover:border-slate-300 rounded-md px-3 py-2 transition-colors"
              onClick={() => setFiltersExpanded((expanded) => !expanded)}
              aria-expanded={filtersExpanded}
            >
              <span className="flex items-center gap-2">
                <Chevron direction={filtersExpanded ? 'down' : 'right'} />
                <span className="text-sm font-medium text-slate-700">Filters</span>
                {filtersHaveSelection && (
                  <span className="text-xs text-blue-700 bg-blue-50 px-1.5 py-0.5 rounded">
                    active
                  </span>
                )}
              </span>
              <span className="text-xs text-slate-500">{filterSummaryText}</span>
            </button>

            {filtersExpanded && (
              <div className="mt-3 space-y-4">
                <ChipFilterRow
                  label="Models"
                  allValues={allModels}
                  selection={selectedModels}
                  renderLabel={modelDisplayName}
                  onToggle={(value) =>
                    setSelectedModels((current) => toggleSelection(current, value, allModels))
                  }
                  onSelectAll={() => setSelectedModels(null)}
                  onSelectNone={() => setSelectedModels(new Set())}
                />

                <ChipFilterRow
                  label="Token types"
                  allValues={allTokenTypes}
                  selection={selectedTokenTypes}
                  renderLabel={tokenTypeDisplayName}
                  onToggle={(value) =>
                    setSelectedTokenTypes((current) =>
                      toggleSelection(current, value, allTokenTypes),
                    )
                  }
                  onSelectAll={() => setSelectedTokenTypes(null)}
                  onSelectNone={() => setSelectedTokenTypes(new Set())}
                />

                <ChipFilterRow
                  label={`Projects${allProjectIds.length > 0 ? ` (${allProjectIds.length})` : ''}`}
                  allValues={allProjectIds}
                  selection={selectedProjects}
                  renderLabel={projectShortName}
                  renderTitle={(value) => value}
                  onToggle={(value) =>
                    setSelectedProjects((current) =>
                      toggleSelection(current, value, allProjectIds),
                    )
                  }
                  onSelectAll={() => setSelectedProjects(null)}
                  onSelectNone={() => setSelectedProjects(new Set())}
                />
              </div>
            )}
          </div>

          {/* Stack-by + view-mode radios */}
          <div className="flex flex-wrap items-center gap-6 pt-1 border-t border-slate-100">
            <RadioGroup
              label="Stack by"
              value={stackBy}
              onChange={setStackBy}
              options={[
                { value: 'model', label: 'Model' },
                { value: 'token-type', label: 'Token type' },
              ]}
            />
            <RadioGroup
              label="Tokens"
              value={viewMode}
              onChange={setViewMode}
              options={[
                { value: 'all', label: 'All (raw count)' },
                { value: 'billable', label: 'Billable' },
              ]}
            />
          </div>

          {chartConfig.hiddenInBillable.length > 0 && (
            <div className="text-xs text-slate-500">
              Hidden in billable view (no pricing entry):{' '}
              {chartConfig.hiddenInBillable.map((m) => modelDisplayName(m)).join(', ')}
            </div>
          )}

          <div>
            <h2 className="text-base font-medium mb-3">
              {viewMode === 'billable' ? 'Daily billable tokens' : 'Daily token usage'} ·{' '}
              {stackBy === 'model' ? 'stacked by model' : 'stacked by token type'}
            </h2>

            <div className="h-80">
              {dailyState.status === 'loading' && (
                <div className="flex h-full items-center justify-center text-sm text-slate-500">
                  Loading…
                </div>
              )}
              {dailyState.status === 'error' && (
                <div className="flex h-full flex-col items-center justify-center text-sm text-slate-500">
                  <p>Could not reach the tokenscale API.</p>
                  <p className="text-xs mt-1">{dailyState.message}</p>
                  <p className="text-xs mt-3">
                    Run <code className="bg-slate-100 px-1 rounded">tokenscale serve</code> in
                    another shell.
                  </p>
                </div>
              )}
              {dailyState.status === 'ok' && chartConfig.rows.length === 0 && (
                <div className="flex h-full items-center justify-center text-sm text-slate-500">
                  No usage matches the current filters.
                </div>
              )}
              {dailyState.status === 'ok' &&
                chartConfig.rows.length > 0 &&
                chartConfig.series.length === 0 && (
                  <div className="flex h-full items-center justify-center text-sm text-slate-500">
                    Every series is filtered out — re-enable a model or token type to render.
                  </div>
                )}
              {dailyState.status === 'ok' &&
                chartConfig.rows.length > 0 &&
                chartConfig.series.length > 0 && (
                  <ResponsiveContainer width="100%" height="100%">
                    <AreaChart
                      data={chartConfig.rows}
                      margin={{ top: 8, right: 16, left: 8, bottom: 0 }}
                    >
                      <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
                      <XAxis dataKey="date" tick={{ fontSize: 12 }} />
                      <YAxis
                        tick={{ fontSize: 12 }}
                        tickFormatter={formatCompactNumber}
                        width={56}
                      />
                      <Tooltip
                        formatter={(rawValue, displayLabel) => [
                          typeof rawValue === 'number'
                            ? rawValue.toLocaleString()
                            : String(rawValue),
                          displayLabel,
                        ]}
                      />
                      <Legend />
                      {chartConfig.series.map((series) => (
                        <Area
                          key={series.key}
                          type="monotone"
                          dataKey={series.key}
                          name={series.displayName}
                          stackId="1"
                          stroke={series.color}
                          fill={series.color}
                          fillOpacity={0.6}
                        />
                      ))}
                    </AreaChart>
                  </ResponsiveContainer>
                )}
            </div>
          </div>
        </section>
      </main>
    </div>
  )
}

// ---------------------------------------------------------------------------
// ChipFilterRow — multi-select chips with Select all / Select none
// ---------------------------------------------------------------------------

type ChipFilterRowProps = {
  label: string
  allValues: string[]
  selection: Selection
  renderLabel: (value: string) => string
  renderTitle?: (value: string) => string
  onToggle: (value: string) => void
  onSelectAll: () => void
  onSelectNone: () => void
}

function ChipFilterRow({
  label,
  allValues,
  selection,
  renderLabel,
  renderTitle,
  onToggle,
  onSelectAll,
  onSelectNone,
}: ChipFilterRowProps) {
  if (allValues.length === 0) {
    return (
      <div>
        <div className="text-xs font-medium text-slate-600 mb-1">{label}</div>
        <div className="text-xs text-slate-400 italic">No values in window.</div>
      </div>
    )
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-1.5">
        <div className="text-xs font-medium text-slate-600">{label}</div>
        <div className="flex items-center gap-3 text-xs">
          <button
            type="button"
            className="text-blue-600 hover:underline disabled:text-slate-300 disabled:no-underline disabled:cursor-not-allowed"
            onClick={onSelectAll}
            disabled={selection === null}
          >
            Select all
          </button>
          <span className="text-slate-300">|</span>
          <button
            type="button"
            className="text-blue-600 hover:underline disabled:text-slate-300 disabled:no-underline disabled:cursor-not-allowed"
            onClick={onSelectNone}
            disabled={selection !== null && selection.size === 0}
          >
            Select none
          </button>
        </div>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {allValues.map((value) => {
          const selected = isSelected(selection, value)
          return (
            <button
              key={value}
              type="button"
              title={renderTitle ? renderTitle(value) : undefined}
              onClick={() => onToggle(value)}
              className={
                'px-2.5 py-1 rounded-full text-xs border transition-colors ' +
                (selected
                  ? 'bg-blue-50 border-blue-400 text-blue-700 hover:bg-blue-100'
                  : 'bg-white border-slate-300 text-slate-400 hover:bg-slate-50 hover:text-slate-600')
              }
            >
              {renderLabel(value)}
            </button>
          )
        })}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// RadioGroup — pill-style radio control
// ---------------------------------------------------------------------------

type RadioGroupProps<T extends string> = {
  label: string
  value: T
  onChange: (value: T) => void
  options: Array<{ value: T; label: string }>
}

function RadioGroup<T extends string>({ label, value, onChange, options }: RadioGroupProps<T>) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-xs font-medium text-slate-600">{label}:</span>
      <div className="inline-flex rounded-md border border-slate-300 overflow-hidden">
        {options.map((option) => {
          const active = option.value === value
          return (
            <button
              key={option.value}
              type="button"
              onClick={() => onChange(option.value)}
              className={
                'px-3 py-1 text-xs transition-colors border-l border-slate-200 first:border-l-0 ' +
                (active
                  ? 'bg-blue-50 text-blue-700'
                  : 'bg-white text-slate-500 hover:bg-slate-50')
              }
            >
              {option.label}
            </button>
          )
        })}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Chevron — small inline SVG so we don't pull in an icon library
// ---------------------------------------------------------------------------

function Chevron({ direction }: { direction: 'right' | 'down' }) {
  const path = direction === 'down' ? 'M5 8l5 5 5-5' : 'M8 5l5 5-5 5'
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="text-slate-500"
      aria-hidden="true"
    >
      <path d={path} />
    </svg>
  )
}
