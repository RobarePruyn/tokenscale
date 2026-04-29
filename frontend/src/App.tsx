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

import { useEffect, useMemo, useState, type FormEvent } from 'react'
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  Line,
  LineChart,
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

type Granularity = 'day' | 'week' | 'month'

type ModelPricingForResponse = {
  input_usd_per_mtok: number
}

type DailyUsageResponse = {
  rows: DailyUsageRow[]
  models: string[]
  tokenTypes: string[]
  modelsWithoutPricing: string[]
  granularity: Granularity
  /** Per-model `input_usd_per_mtok` so the frontend can convert `billable`
   *  values to USD on the fly. Models without a pricing entry are absent. */
  pricingByModel: Record<string, ModelPricingForResponse>
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

type SubscriptionDto = {
  id: number
  plan_name: string
  monthly_usd: number
  started_at: string
  ended_at: string | null
}

type SubscriptionsResponse = {
  subscriptions: SubscriptionDto[]
}

// ---------------------------------------------------------------------------
// View-state types
// ---------------------------------------------------------------------------

type StackBy = 'model' | 'token-type'

/** Three counting modes:
 *  - `all`      raw token counts, all categories weighted equally
 *  - `billable` weighted by API-price multipliers, in input-token-equivalent units
 *  - `cost`     billable × per-model input price ÷ 1M, in USD
 *
 *  Both `billable` and `cost` require a pricing entry for the model;
 *  `cost` additionally needs the per-model `input_usd_per_mtok`. Models
 *  without pricing are hidden from those views.
 */
type ViewMode = 'all' | 'billable' | 'cost'
type RangePreset = '7d' | '30d' | '90d' | '365d' | 'all' | 'custom'

/** "auto" defers to a window-length heuristic; the rest map 1:1 to the
 *  server's `?granularity=` values.
 */
type GranularityChoice = 'auto' | 'day' | 'week' | 'month'

type ChartType = 'area' | 'bar' | 'line'
type YAxisScale = 'linear' | 'log'

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

const MILLIS_PER_DAY = 1000 * 60 * 60 * 24

/** Choose a sensible default granularity for a window of `daysInWindow`
 *  calendar days. Tuned so the chart never has more than ~60 buckets:
 *  daily up to 60 days, weekly to a year, monthly beyond that.
 */
function autoGranularity(daysInWindow: number): Granularity {
  if (daysInWindow <= 60) return 'day'
  if (daysInWindow <= 365) return 'week'
  return 'month'
}

function daysBetween(fromDate: string, toDate: string): number {
  const fromMillis = Date.parse(`${fromDate}T00:00:00Z`)
  const toMillis = Date.parse(`${toDate}T00:00:00Z`)
  if (Number.isNaN(fromMillis) || Number.isNaN(toMillis)) return 0
  return Math.max(0, Math.round((toMillis - fromMillis) / MILLIS_PER_DAY))
}

const SHORT_MONTH_NAMES = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
]

/** Format a YYYY-MM-DD bucket label for display on the x-axis. The format
 *  depends on the bucket size: day/week show "Apr 27"; month shows "Apr
 *  2026" so the year is always visible at month resolution.
 */
function formatBucketLabel(bucketIsoDate: string, granularity: Granularity): string {
  const parts = bucketIsoDate.split('-')
  if (parts.length !== 3) return bucketIsoDate
  const year = parts[0]
  const monthIndex = Number.parseInt(parts[1], 10) - 1
  const day = Number.parseInt(parts[2], 10)
  if (Number.isNaN(monthIndex) || Number.isNaN(day)) return bucketIsoDate
  const monthName = SHORT_MONTH_NAMES[monthIndex] ?? parts[1]
  switch (granularity) {
    case 'month':
      return `${monthName} ${year}`
    case 'week':
    case 'day':
      return `${monthName} ${day}`
  }
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

/** Compact dollar y-axis labels — "$1.2M", "$500", "$0.05".
 *  Falls back to two-decimal precision below $1 so small days don't show
 *  as "$0".
 */
function formatCompactDollars(value: number): string {
  const absolute = Math.abs(value)
  if (absolute >= 1e9) return `$${stripTrailingZero((value / 1e9).toFixed(1))}B`
  if (absolute >= 1e6) return `$${stripTrailingZero((value / 1e6).toFixed(1))}M`
  if (absolute >= 1e3) return `$${stripTrailingZero((value / 1e3).toFixed(1))}K`
  if (absolute >= 1) return `$${value.toFixed(0)}`
  return `$${value.toFixed(2)}`
}

/** Tooltip-friendly dollar formatter — "$1,234.56". */
function formatExactDollars(value: number): string {
  return `$${value.toLocaleString('en-US', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`
}

function tokenFieldsForView(
  modelTokens: ModelTokens,
  viewMode: ViewMode,
  modelPricing: ModelPricingForResponse | undefined,
): BillableBreakdown {
  if (viewMode === 'cost' && modelTokens.billable && modelPricing) {
    // billable_value × $/MTok ÷ 1e6 = $ for that token type
    const dollarsPerBillableUnit = modelPricing.input_usd_per_mtok / 1_000_000
    return {
      input: modelTokens.billable.input * dollarsPerBillableUnit,
      output: modelTokens.billable.output * dollarsPerBillableUnit,
      cache_read: modelTokens.billable.cache_read * dollarsPerBillableUnit,
      cache_write_5m: modelTokens.billable.cache_write_5m * dollarsPerBillableUnit,
      cache_write_1h: modelTokens.billable.cache_write_1h * dollarsPerBillableUnit,
    }
  }
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

const AVERAGE_DAYS_PER_MONTH = 30.4375 // 365.25 / 12

/** Pro-rate each subscription by its overlap with the chart's window.
 *  A subscription that was active for half the window contributes half a
 *  month of its `monthly_usd`. Subs that don't overlap contribute zero.
 */
function subscriptionCostOverWindow(
  subscriptions: SubscriptionDto[],
  fromDate: string,
  toDate: string,
): number {
  const windowFromMs = Date.parse(`${fromDate}T00:00:00Z`)
  const windowToMs = Date.parse(`${toDate}T23:59:59.999Z`)
  if (Number.isNaN(windowFromMs) || Number.isNaN(windowToMs)) return 0

  let total = 0
  for (const sub of subscriptions) {
    const subFromMs = Date.parse(`${sub.started_at}T00:00:00Z`)
    const subToMs = sub.ended_at
      ? Date.parse(`${sub.ended_at}T23:59:59.999Z`)
      : Number.POSITIVE_INFINITY
    if (Number.isNaN(subFromMs)) continue

    const overlapStart = Math.max(windowFromMs, subFromMs)
    const overlapEnd = Math.min(windowToMs, subToMs)
    if (overlapEnd <= overlapStart) continue

    const overlapDays = (overlapEnd - overlapStart) / MILLIS_PER_DAY
    total += sub.monthly_usd * (overlapDays / AVERAGE_DAYS_PER_MONTH)
  }
  return total
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

  const [granularityChoice, setGranularityChoice] = useState<GranularityChoice>('auto')
  const [chartType, setChartType] = useState<ChartType>('area')
  const [yAxisScale, setYAxisScale] = useState<YAxisScale>('linear')

  // Effective bucket size — auto = pick from window length, else honor
  // the user's explicit choice. Server gets a concrete day/week/month.
  const effectiveGranularity: Granularity =
    granularityChoice === 'auto'
      ? autoGranularity(daysBetween(fromDate, toDate))
      : granularityChoice

  const [healthState, setHealthState] = useState<FetchState<HealthResponse>>({ status: 'idle' })
  const [projectsState, setProjectsState] = useState<FetchState<ProjectsResponse>>({
    status: 'idle',
  })
  const [dailyState, setDailyState] = useState<FetchState<DailyUsageResponse>>({ status: 'idle' })
  const [subscriptionsState, setSubscriptionsState] = useState<FetchState<SubscriptionsResponse>>({
    status: 'idle',
  })

  // Bumped after a subscription mutation so the GET re-runs.
  const [subscriptionsRevision, setSubscriptionsRevision] = useState(0)
  const refreshSubscriptions = () => setSubscriptionsRevision((current) => current + 1)

  // Subscriptions — re-fetched on mount and after any CRUD mutation.
  useEffect(() => {
    let cancelled = false
    setSubscriptionsState({ status: 'loading' })
    fetch('/api/v1/subscriptions')
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as SubscriptionsResponse
      })
      .then((data) => {
        if (!cancelled) setSubscriptionsState({ status: 'ok', data })
      })
      .catch((error) => {
        if (!cancelled) {
          setSubscriptionsState({ status: 'error', message: (error as Error).message })
        }
      })
    return () => {
      cancelled = true
    }
  }, [subscriptionsRevision])

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
    const params = new URLSearchParams({
      from: fromDate,
      to: toDate,
      provider: providerFilter,
      granularity: effectiveGranularity,
    })
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
  }, [providerFilter, selectedProjects, fromDate, toDate, effectiveGranularity])

  // ----- Derived chart config ---------------------------------------------
  const chartConfig = useMemo(() => {
    if (dailyState.status !== 'ok') {
      return { rows: [], series: [], hiddenInPricedView: [] as string[] }
    }
    const data = dailyState.data
    const visibleTokenTypes = new Set(
      data.tokenTypes.filter((t) => isSelected(selectedTokenTypes, t)),
    )
    let visibleModels = data.models.filter((m) => isSelected(selectedModels, m))
    // Both 'billable' and 'cost' need a pricing entry — same gating, hide
    // any unpriced model from the chart and surface the list as a footnote.
    const hiddenInPricedView: string[] = []
    if (viewMode === 'billable' || viewMode === 'cost') {
      visibleModels = visibleModels.filter((m) => {
        if (data.modelsWithoutPricing.includes(m)) {
          hiddenInPricedView.push(m)
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
            ? sumSelectedTokenFields(
                tokenFieldsForView(tokens, viewMode, data.pricingByModel[modelId]),
                visibleTokenTypes,
              )
            : 0
        }
        return cells
      })
      return { rows, series, hiddenInPricedView }
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
          sum += tokenFieldsForView(tokens, viewMode, data.pricingByModel[modelId])[
            tokenType as keyof BillableBreakdown
          ]
        }
        cells[tokenType] = sum
      }
      return cells
    })
    return { rows, series, hiddenInPricedView }
  }, [dailyState, selectedModels, selectedTokenTypes, stackBy, viewMode])

  // Counterfactual cost = "what these tokens would have cost on the API at
  // list rates" — sum of `billable.X × input_price ÷ 1e6` across the chart's
  // currently visible cells. Reflects the user's full filter set so the
  // headline number always matches what's drawn.
  const counterfactualCostUsd = useMemo(() => {
    if (dailyState.status !== 'ok') return null
    const data = dailyState.data
    const visibleTokenTypes = new Set(
      data.tokenTypes.filter((t) => isSelected(selectedTokenTypes, t)),
    )
    const visibleModels = data.models.filter((m) => isSelected(selectedModels, m))
    let total = 0
    for (const row of data.rows) {
      for (const modelId of visibleModels) {
        const tokens = row.byModel[modelId]
        const pricing = data.pricingByModel[modelId]
        if (!tokens || !tokens.billable || !pricing) continue
        const dollarsPerBillableUnit = pricing.input_usd_per_mtok / 1_000_000
        if (visibleTokenTypes.has('input'))
          total += tokens.billable.input * dollarsPerBillableUnit
        if (visibleTokenTypes.has('output'))
          total += tokens.billable.output * dollarsPerBillableUnit
        if (visibleTokenTypes.has('cache_read'))
          total += tokens.billable.cache_read * dollarsPerBillableUnit
        if (visibleTokenTypes.has('cache_write_5m'))
          total += tokens.billable.cache_write_5m * dollarsPerBillableUnit
        if (visibleTokenTypes.has('cache_write_1h'))
          total += tokens.billable.cache_write_1h * dollarsPerBillableUnit
      }
    }
    return total
  }, [dailyState, selectedModels, selectedTokenTypes])

  // Subscription cost is purely date-window based — your subscription costs
  // what it costs regardless of which models / projects you used.
  const subscriptionCostUsd = useMemo(() => {
    if (subscriptionsState.status !== 'ok') return null
    return subscriptionCostOverWindow(
      subscriptionsState.data.subscriptions,
      fromDate,
      toDate,
    )
  }, [subscriptionsState, fromDate, toDate])

  const netValueUsd =
    counterfactualCostUsd !== null && subscriptionCostUsd !== null
      ? counterfactualCostUsd - subscriptionCostUsd
      : null

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

      {pricingNeedsReview && (viewMode === 'billable' || viewMode === 'cost') && (
        <div className="bg-amber-50 border-b border-amber-200 text-amber-900 text-xs px-6 py-2">
          <span className="font-medium">Pricing needs review:</span>{' '}
          {viewMode === 'cost' ? 'cost figures are' : 'the cost-weighted view is'} using seed
          values from <code className="bg-amber-100 px-1 rounded">pricing.toml</code>. Verify
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
          <div className="flex flex-wrap items-center gap-x-6 gap-y-3 pt-1 border-t border-slate-100">
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
              label="Counting"
              value={viewMode}
              onChange={setViewMode}
              options={[
                { value: 'all', label: 'Raw' },
                { value: 'billable', label: 'Cost-weighted' },
                { value: 'cost', label: 'Cost (USD)' },
              ]}
              labelHelp="Raw counts each token equally. Cost-weighted weights each token type by its Anthropic API price relative to input (output ×5, cache_read ×0.1, cache writes ×1.25/×2). Cost (USD) converts cost-weighted to dollars per the model's input price — what these tokens would have cost on the API. Models without pricing are hidden from the latter two views."
            />
          </div>

          {/* Chart-display controls — granularity / chart type / scale */}
          <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
            <RadioGroup
              label="Granularity"
              value={granularityChoice}
              onChange={setGranularityChoice}
              options={[
                { value: 'auto', label: `Auto (${effectiveGranularity})` },
                { value: 'day', label: 'Day' },
                { value: 'week', label: 'Week' },
                { value: 'month', label: 'Month' },
              ]}
              labelHelp="Auto picks day for windows up to 60 days, week up to a year, month beyond that — chosen so the chart never has more than ~60 buckets."
            />
            <RadioGroup
              label="Chart"
              value={chartType}
              onChange={setChartType}
              options={[
                { value: 'area', label: 'Area' },
                { value: 'bar', label: 'Bar' },
                { value: 'line', label: 'Line' },
              ]}
              labelHelp="Area and Bar stack series so the height is the total. Line shows each series independently — useful when you want to compare trends instead of composition."
            />
            <RadioGroup
              label="Scale"
              value={yAxisScale}
              onChange={setYAxisScale}
              options={[
                { value: 'linear', label: 'Linear' },
                { value: 'log', label: 'Log' },
              ]}
              labelHelp="Log compresses tall series so smaller ones become legible on the same axis. Useful when one model or one token type dominates."
            />
          </div>

          {chartConfig.hiddenInPricedView.length > 0 && (
            <div className="text-xs text-slate-500">
              Hidden ({viewMode === 'cost' ? 'no pricing entry → no cost' : 'no pricing entry'}):{' '}
              {chartConfig.hiddenInPricedView.map((m) => modelDisplayName(m)).join(', ')}
            </div>
          )}

          <StatRow
            counterfactualCostUsd={counterfactualCostUsd}
            subscriptionCostUsd={subscriptionCostUsd}
            netValueUsd={netValueUsd}
            hasSubscriptions={
              subscriptionsState.status === 'ok' &&
              subscriptionsState.data.subscriptions.length > 0
            }
            pricingNeedsReview={pricingNeedsReview}
          />

          <div>
            <h2 className="text-base font-medium mb-3">
              {viewMode === 'cost'
                ? 'Daily cost (USD, API list)'
                : viewMode === 'billable'
                  ? 'Daily cost-weighted tokens'
                  : 'Daily token usage'}{' '}
              · {stackBy === 'model' ? 'stacked by model' : 'stacked by token type'}
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
                    <ChartByType
                      chartType={chartType}
                      data={chartConfig.rows}
                      series={chartConfig.series}
                      yAxisScale={yAxisScale}
                      granularity={dailyState.data.granularity}
                      viewMode={viewMode}
                    />
                  </ResponsiveContainer>
                )}
            </div>
          </div>
        </section>

        <SubscriptionsPanel
          subscriptionsState={subscriptionsState}
          onCreated={refreshSubscriptions}
          onDeleted={refreshSubscriptions}
        />
      </main>
    </div>
  )
}

// ---------------------------------------------------------------------------
// StatRow — three KPIs above the chart: counterfactual cost, subscription
// cost, net value. Renders only when we have a counterfactual figure
// (i.e., the daily endpoint succeeded).
// ---------------------------------------------------------------------------

type StatRowProps = {
  counterfactualCostUsd: number | null
  subscriptionCostUsd: number | null
  netValueUsd: number | null
  hasSubscriptions: boolean
  pricingNeedsReview: boolean
}

function StatRow({
  counterfactualCostUsd,
  subscriptionCostUsd,
  netValueUsd,
  hasSubscriptions,
  pricingNeedsReview,
}: StatRowProps) {
  if (counterfactualCostUsd === null) return null

  const counterfactualLabel = pricingNeedsReview
    ? 'Counterfactual API cost (approx)'
    : 'Counterfactual API cost'

  return (
    <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
      <StatCard
        label={counterfactualLabel}
        value={formatExactDollars(counterfactualCostUsd)}
        helpText="What these tokens would have cost on the Anthropic API at list rates, summed across the chart's currently visible cells."
      />
      <StatCard
        label="Subscriptions paid in window"
        value={
          subscriptionCostUsd === null
            ? '—'
            : formatExactDollars(subscriptionCostUsd)
        }
        helpText={
          hasSubscriptions
            ? 'Sum of declared subscriptions, pro-rated by the days each one overlaps the chart window.'
            : 'No subscriptions declared yet. Add one below to see the net value of your plan.'
        }
        muted={!hasSubscriptions}
      />
      <StatCard
        label="Net value"
        value={
          netValueUsd === null
            ? '—'
            : formatExactDollars(netValueUsd)
        }
        helpText="Counterfactual API cost minus subscriptions paid. Positive means your subscription is cheaper than running the same usage on the API."
        emphasize={netValueUsd !== null && netValueUsd > 0}
      />
    </div>
  )
}

type StatCardProps = {
  label: string
  value: string
  helpText: string
  muted?: boolean
  emphasize?: boolean
}

function StatCard({ label, value, helpText, muted, emphasize }: StatCardProps) {
  return (
    <div
      className={
        'rounded-md border px-4 py-3 ' +
        (emphasize
          ? 'border-emerald-300 bg-emerald-50'
          : muted
            ? 'border-slate-200 bg-slate-50'
            : 'border-slate-200 bg-white')
      }
    >
      <div
        className={
          'text-xs font-medium cursor-help underline decoration-dotted decoration-slate-400 ' +
          (muted ? 'text-slate-400' : 'text-slate-500')
        }
        title={helpText}
      >
        {label}
      </div>
      <div
        className={
          'mt-1 text-xl font-semibold tabular-nums ' +
          (emphasize ? 'text-emerald-700' : muted ? 'text-slate-400' : 'text-slate-900')
        }
      >
        {value}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// SubscriptionsPanel — list, add (inline form), delete.
// ---------------------------------------------------------------------------

type SubscriptionsPanelProps = {
  subscriptionsState: FetchState<SubscriptionsResponse>
  onCreated: () => void
  onDeleted: () => void
}

function SubscriptionsPanel({
  subscriptionsState,
  onCreated,
  onDeleted,
}: SubscriptionsPanelProps) {
  const [showForm, setShowForm] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const subscriptions =
    subscriptionsState.status === 'ok' ? subscriptionsState.data.subscriptions : []

  async function submitForm(formData: {
    planName: string
    monthlyUsd: number
    startedAt: string
    endedAt: string | null
  }) {
    setSubmitting(true)
    setFormError(null)
    try {
      const response = await fetch('/api/v1/subscriptions', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          plan_name: formData.planName,
          monthly_usd: formData.monthlyUsd,
          started_at: formData.startedAt,
          ended_at: formData.endedAt,
        }),
      })
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: { message?: string }
        }
        throw new Error(body.error?.message ?? `HTTP ${response.status}`)
      }
      setShowForm(false)
      onCreated()
    } catch (error) {
      setFormError((error as Error).message)
    } finally {
      setSubmitting(false)
    }
  }

  async function deleteSubscription(id: number) {
    try {
      const response = await fetch(`/api/v1/subscriptions/${id}`, { method: 'DELETE' })
      if (!response.ok && response.status !== 204) {
        throw new Error(`HTTP ${response.status}`)
      }
      onDeleted()
    } catch (error) {
      // For Phase 1, surface deletion errors via alert() — they're rare and
      // don't warrant a dedicated error UI yet.
      window.alert(`Could not delete subscription: ${(error as Error).message}`)
    }
  }

  return (
    <section className="bg-white rounded-lg border border-slate-200 p-5 space-y-4">
      <div className="flex items-baseline justify-between">
        <h2 className="text-base font-medium">Subscriptions</h2>
        {!showForm && (
          <button
            type="button"
            className="text-sm text-blue-600 hover:underline"
            onClick={() => {
              setShowForm(true)
              setFormError(null)
            }}
          >
            + Add subscription
          </button>
        )}
      </div>

      {showForm && (
        <SubscriptionForm
          submitting={submitting}
          error={formError}
          onCancel={() => {
            setShowForm(false)
            setFormError(null)
          }}
          onSubmit={submitForm}
        />
      )}

      {subscriptionsState.status === 'loading' && (
        <div className="text-sm text-slate-500">Loading subscriptions…</div>
      )}
      {subscriptionsState.status === 'error' && (
        <div className="text-sm text-rose-700">
          Could not load subscriptions: {subscriptionsState.message}
        </div>
      )}
      {subscriptionsState.status === 'ok' && subscriptions.length === 0 && !showForm && (
        <div className="text-sm text-slate-500">
          No subscriptions yet. Add Claude Max, a Team seat, or any other flat-fee plan to see net
          value over your selected date range.
        </div>
      )}
      {subscriptions.length > 0 && (
        <ul className="divide-y divide-slate-100">
          {subscriptions.map((sub) => (
            <li key={sub.id} className="flex items-center justify-between py-2 text-sm">
              <div>
                <span className="font-medium text-slate-900">{sub.plan_name}</span>
                <span className="text-slate-500">
                  {' · '}
                  {formatExactDollars(sub.monthly_usd)}/mo
                </span>
                <span className="text-slate-500">
                  {' · '}
                  {sub.ended_at ? `${sub.started_at} → ${sub.ended_at}` : `since ${sub.started_at}`}
                </span>
              </div>
              <button
                type="button"
                className="text-xs text-rose-600 hover:underline"
                onClick={() => {
                  if (window.confirm(`Delete subscription "${sub.plan_name}"?`)) {
                    void deleteSubscription(sub.id)
                  }
                }}
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

type SubscriptionFormProps = {
  submitting: boolean
  error: string | null
  onCancel: () => void
  onSubmit: (data: {
    planName: string
    monthlyUsd: number
    startedAt: string
    endedAt: string | null
  }) => void
}

function SubscriptionForm({ submitting, error, onCancel, onSubmit }: SubscriptionFormProps) {
  const [planName, setPlanName] = useState('Claude Max')
  const [monthlyUsd, setMonthlyUsd] = useState<string>('200')
  const [startedAt, setStartedAt] = useState<string>(() => isoDateDaysAgo(0))
  const [endedAt, setEndedAt] = useState<string>('')

  function handleSubmit(event: FormEvent) {
    event.preventDefault()
    const parsedAmount = Number.parseFloat(monthlyUsd)
    if (!Number.isFinite(parsedAmount) || parsedAmount < 0) {
      window.alert('Monthly USD must be a non-negative number.')
      return
    }
    onSubmit({
      planName: planName.trim(),
      monthlyUsd: parsedAmount,
      startedAt,
      endedAt: endedAt || null,
    })
  }

  return (
    <form
      className="rounded-md border border-slate-200 bg-slate-50 p-4 space-y-3"
      onSubmit={handleSubmit}
    >
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        <label className="block">
          <span className="text-xs font-medium text-slate-600">Plan name</span>
          <input
            type="text"
            required
            value={planName}
            onChange={(event) => setPlanName(event.target.value)}
            className="mt-1 w-full border border-slate-300 rounded-md px-2 py-1 text-sm bg-white"
          />
        </label>
        <label className="block">
          <span className="text-xs font-medium text-slate-600">Monthly (USD)</span>
          <input
            type="number"
            required
            min="0"
            step="0.01"
            value={monthlyUsd}
            onChange={(event) => setMonthlyUsd(event.target.value)}
            className="mt-1 w-full border border-slate-300 rounded-md px-2 py-1 text-sm bg-white"
          />
        </label>
        <label className="block">
          <span className="text-xs font-medium text-slate-600">Started</span>
          <input
            type="date"
            required
            value={startedAt}
            onChange={(event) => setStartedAt(event.target.value)}
            className="mt-1 w-full border border-slate-300 rounded-md px-2 py-1 text-sm bg-white"
          />
        </label>
        <label className="block">
          <span className="text-xs font-medium text-slate-600">Ended (optional)</span>
          <input
            type="date"
            min={startedAt}
            value={endedAt}
            onChange={(event) => setEndedAt(event.target.value)}
            className="mt-1 w-full border border-slate-300 rounded-md px-2 py-1 text-sm bg-white"
          />
        </label>
      </div>

      {error && <div className="text-xs text-rose-700">{error}</div>}

      <div className="flex items-center gap-2">
        <button
          type="submit"
          disabled={submitting}
          className="bg-blue-600 hover:bg-blue-700 text-white text-sm rounded-md px-3 py-1 disabled:bg-slate-300"
        >
          {submitting ? 'Saving…' : 'Save'}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="text-sm text-slate-600 hover:underline"
        >
          Cancel
        </button>
      </div>
    </form>
  )
}

// ---------------------------------------------------------------------------
// ChartByType — render the appropriate Recharts chart for `chartType`.
// Area/Bar stack series; Line shows them independently. Shared shell
// (axes + tooltip + legend) is factored into a single fragment so each
// chart-type branch is just the outer chart component plus per-series
// children.
// ---------------------------------------------------------------------------

type ChartSeries = { key: string; displayName: string; color: string }

type ChartByTypeProps = {
  chartType: ChartType
  data: Array<Record<string, string | number>>
  series: ChartSeries[]
  yAxisScale: YAxisScale
  granularity: Granularity
  viewMode: ViewMode
}

function ChartByType({
  chartType,
  data,
  series,
  yAxisScale,
  granularity,
  viewMode,
}: ChartByTypeProps) {
  const chartMargin = { top: 8, right: 16, left: 8, bottom: 0 }
  const isCostMode = viewMode === 'cost'

  // Log-scale floor: $1 in cost mode (sub-$1 days clamp here), 1 token
  // otherwise. `allowDataOverflow` keeps clamped values visible at the
  // floor rather than vanishing.
  const yAxisDomain: [number | string, number | string] =
    yAxisScale === 'log' ? [isCostMode ? 0.01 : 1, 'auto'] : ['auto', 'auto']

  const yTickFormatter = isCostMode ? formatCompactDollars : formatCompactNumber
  const tooltipValueFormatter = (rawValue: unknown): string => {
    if (typeof rawValue !== 'number') return String(rawValue)
    return isCostMode ? formatExactDollars(rawValue) : rawValue.toLocaleString()
  }
  const yAxisWidth = isCostMode ? 64 : 56

  const sharedShell = (
    <>
      <CartesianGrid strokeDasharray="3 3" stroke="#e2e8f0" />
      <XAxis
        dataKey="date"
        tick={{ fontSize: 12 }}
        tickFormatter={(value: string) => formatBucketLabel(value, granularity)}
      />
      <YAxis
        tick={{ fontSize: 12 }}
        tickFormatter={yTickFormatter}
        width={yAxisWidth}
        scale={yAxisScale}
        domain={yAxisDomain}
        allowDataOverflow={yAxisScale === 'log'}
      />
      <Tooltip
        labelFormatter={(label) =>
          typeof label === 'string' ? formatBucketLabel(label, granularity) : String(label)
        }
        formatter={(rawValue, displayLabel) => [tooltipValueFormatter(rawValue), displayLabel]}
      />
      <Legend />
    </>
  )

  if (chartType === 'area') {
    return (
      <AreaChart data={data} margin={chartMargin}>
        {sharedShell}
        {series.map((s) => (
          <Area
            key={s.key}
            type="monotone"
            dataKey={s.key}
            name={s.displayName}
            stackId="1"
            stroke={s.color}
            fill={s.color}
            fillOpacity={0.6}
          />
        ))}
      </AreaChart>
    )
  }

  if (chartType === 'bar') {
    return (
      <BarChart data={data} margin={chartMargin}>
        {sharedShell}
        {series.map((s) => (
          <Bar key={s.key} dataKey={s.key} name={s.displayName} stackId="1" fill={s.color} />
        ))}
      </BarChart>
    )
  }

  // chartType === 'line' — lines aren't stacked; each series shows its
  // absolute value over time, which is the natural "compare trends" view.
  return (
    <LineChart data={data} margin={chartMargin}>
      {sharedShell}
      {series.map((s) => (
        <Line
          key={s.key}
          type="monotone"
          dataKey={s.key}
          name={s.displayName}
          stroke={s.color}
          strokeWidth={2}
          dot={false}
        />
      ))}
    </LineChart>
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
  /** Optional help text — when present, the label is rendered with a cursor
   *  hint and a native browser tooltip (`title` attribute) carrying the text.
   */
  labelHelp?: string
}

function RadioGroup<T extends string>({
  label,
  value,
  onChange,
  options,
  labelHelp,
}: RadioGroupProps<T>) {
  return (
    <div className="flex items-center gap-2">
      <span
        className={
          'text-xs font-medium text-slate-600 ' +
          (labelHelp ? 'cursor-help underline decoration-dotted decoration-slate-400' : '')
        }
        title={labelHelp}
      >
        {label}:
      </span>
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
