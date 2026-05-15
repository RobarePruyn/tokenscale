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
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
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

type ModelImpact = {
  energy_wh: number
  facility_wh: number
  co2eG: number | null
  waterL: number | null
  // Energy-side ± band (model-factor uncertainty only; PUE folds in).
  maxUncertaintyPct: number
  // Combined ± bands for CO₂e and water — model + grid via quadrature.
  co2eUncertaintyPct: number
  waterUncertaintyPct: number
  eventsMissingEnvFactor: number
  eventsUsingFallbackPue: number
  eventsUsingFallbackWue: number
  eventsCount: number
}

type ModelTokens = {
  input: number
  output: number
  cache_read: number
  cache_write_5m: number
  cache_write_1h: number
  billable?: BillableBreakdown
  billable_total?: number
  /** Per-(bucket, model) environmental impact, time-anchored against
   *  factor rows authoritative at each event's `occurred_at`. Always
   *  present when the cell has events; co2eG/waterL may be null when
   *  no event in the bucket had a usable grid factor. */
  impact: ModelImpact
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
  /** Models present in the data but missing from the env_factor file —
   *  environmental views render these as "factor data unavailable". */
  modelsWithoutFactors: string[]
  /** Configured AWS region the impact figures are attributed to. */
  configuredRegion: string
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
    /** Most recent `source_accessed_at` across loaded models. The
     *  dashboard surfaces this in the banner so the user can decide
     *  whether the values are too stale to rely on. */
    accessed_at: string | null
  }
  environmental: {
    schema_version: number
    file_status: string
    file_version: string | null
    file_published: string | null
    methodology: string | null
    methodology_source: string | null
    model_count: number
    region_count: number
    is_placeholder: boolean
    needs_review: boolean
    accessed_at: string | null
    configured_region: string
    configured_region_egrid_subregion: string | null
    configured_region_egrid_subregion_full_name: string | null
  }
  ingest: {
    /** ISO-8601 UTC timestamp of the most recent file scan, or null
     *  if no scan has run yet. Drives the dashboard freshness chip. */
    last_scanned_at: string | null
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
// Active factor provenance — what factor rows the dashboard currently
// resolves against. Fetched from /api/v1/factors/active once on mount.
// Lets the Sources panel show "this Energy number rests on these
// specific (model, region) rows from environmental-factors.toml."
// ---------------------------------------------------------------------------

type ModelFactorEntry = {
  provider: string
  model_id: string
  display_name: string
  released_at: string | null
  valid_from: string | null
  source_doc: string | null
  confidence: string | null
  uncertainty_range_pct: number | null
  wh_per_mtok_input: number | null
  wh_per_mtok_output: number | null
  wh_per_mtok_cache_read: number | null
  wh_per_mtok_cache_write_5m: number | null
  wh_per_mtok_cache_write_1h: number | null
  notes: string | null
}

type GridFactorEntry = {
  region_id: string
  display_name: string
  valid_from: string | null
  co2e_kg_per_kwh: number | null
  co2e_uncertainty_range_pct: number | null
  water_l_per_kwh: number | null
  water_uncertainty_range_pct: number | null
  pue: number | null
  egrid_subregion: string | null
  egrid_subregion_full_name: string | null
  source_url_co2e: string | null
  source_accessed_at: string | null
  notes: string | null
}

type ActiveFactorsResponse = {
  models: ModelFactorEntry[]
  regions: GridFactorEntry[]
  configured_region: string
  file_version: string | null
  methodology: string | null
  methodology_source: string | null
}

// ---------------------------------------------------------------------------
// Billing types — Stripe CSV import preview/commit + historical list.
// ---------------------------------------------------------------------------

type BillingCategory = 'subscription' | 'overage' | 'one_time' | 'refund' | 'unknown'

const BILLING_CATEGORY_OPTIONS: ReadonlyArray<{ value: BillingCategory; label: string }> = [
  { value: 'subscription', label: 'Subscription' },
  { value: 'overage', label: 'Overage' },
  { value: 'one_time', label: 'One-time' },
  { value: 'refund', label: 'Refund' },
  { value: 'unknown', label: 'Unknown' },
]

type PreviewedCharge = {
  source: string
  occurred_at: string
  amount_usd: number
  description: string
  category: BillingCategory
  external_id: string | null
  raw: string | null
}

type ConflictingSubscription = {
  id: number
  plan_name: string
  monthly_usd: number
  started_at: string
  ended_at: string | null
  overlapping_charge_external_ids: string[]
}

type BillingPreviewResponse = {
  charges: PreviewedCharge[]
  conflicting_subscriptions: ConflictingSubscription[]
  skipped_non_usd_count: number
  total_amount_usd: number
}

type BillingCommitResponse = {
  inserted: number
  skipped_duplicate: number
  dismissed_subscriptions: number
}

type BillingChargeRow = {
  id: number
  source: string
  occurred_at: string
  amount_usd: number
  description: string | null
  category: string
  external_id: string | null
  created_at: string
}

type BillingChargesResponse = {
  charges: BillingChargeRow[]
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

/** Lower bound for the "All time" preset. ChatGPT's launch on
 *  2022-11-30 is the practical "earliest possible LLM usage" date —
 *  no provider tracked here predates it, so going earlier just produces
 *  empty buckets. Padded to the 1st of December for a clean month
 *  boundary in monthly granularity.
 */
const ALL_TIME_FROM_DATE = '2022-12-01'

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
/// Auto-pick the bucket size based on window length, keeping the
/// visual comparison honest across adjacent window presets:
///   * `≤ 120 days` → daily buckets (so 7d / 30d / 90d all share the
///     same bucket size and the user can visually compare peaks
///     between them without a 7× scale jump from the bucket
///     changing under their feet).
///   * `≤ 730 days` → weekly buckets (1y view; daily would be 365
///     spiky bars across a ~1500px chart, ~4px per bar — unreadable).
///   * `> 730 days` → monthly buckets ("All" view back to ChatGPT
///     launch is ~3.5 years; weekly would be 180+ bars).
///
/// Thresholds were retuned upward (from 60 / 365) after the user
/// reported that 90d / 1y views looked "somewhat broken" — the old
/// boundaries flipped granularity at exactly the points where they
/// got compared against the next preset, magnifying the peak-height
/// mismatch.
function autoGranularity(daysInWindow: number): Granularity {
  if (daysInWindow <= 120) return 'day'
  if (daysInWindow <= 730) return 'week'
  return 'month'
}

/// Render the chart's H2 title for a given (granularity, view-mode)
/// pair. The cadence word in the title MUST match the bucket size
/// shown — otherwise the y-axis values look mis-scaled because the
/// label promises "Daily" but the bars represent a week's or month's
/// worth of tokens.
function chartTitleForGranularity(granularity: Granularity, viewMode: ViewMode): string {
  const cadence =
    granularity === 'month' ? 'Monthly' : granularity === 'week' ? 'Weekly' : 'Daily'
  if (viewMode === 'cost') return `${cadence} cost (USD, API list)`
  if (viewMode === 'billable') return `${cadence} cost-weighted tokens`
  return `${cadence} token usage`
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

/** Enumerate every bucket-start date in `[fromDate, toDate]` for the given
 *  granularity. Used to zero-fill the chart so the full requested window
 *  is visible — sparse data on a wide window should render as a flat
 *  baseline with the data spike where it actually is, not as a
 *  data-only chart with no temporal context.
 *
 *  Bucket semantics match the SQLite expressions in `tokenscale-store`:
 *  - day: each calendar day
 *  - week: Monday-starting weeks; first bucket is the Monday on or before fromDate
 *  - month: each calendar month start
 */
function enumerateBuckets(
  fromDate: string,
  toDate: string,
  granularity: Granularity,
): string[] {
  const result: string[] = []
  const fromMs = Date.parse(`${fromDate}T00:00:00Z`)
  const toMs = Date.parse(`${toDate}T00:00:00Z`)
  if (Number.isNaN(fromMs) || Number.isNaN(toMs) || fromMs > toMs) return result

  const cursor = new Date(fromMs)
  // Snap cursor to bucket start.
  if (granularity === 'week') {
    // getUTCDay(): 0 = Sunday … 6 = Saturday. ISO week starts Monday.
    const weekday = cursor.getUTCDay()
    const offsetToMonday = weekday === 0 ? -6 : 1 - weekday
    cursor.setUTCDate(cursor.getUTCDate() + offsetToMonday)
  } else if (granularity === 'month') {
    cursor.setUTCDate(1)
  }

  while (cursor.getTime() <= toMs) {
    result.push(cursor.toISOString().slice(0, 10))
    if (granularity === 'day') {
      cursor.setUTCDate(cursor.getUTCDate() + 1)
    } else if (granularity === 'week') {
      cursor.setUTCDate(cursor.getUTCDate() + 7)
    } else {
      cursor.setUTCMonth(cursor.getUTCMonth() + 1)
    }
  }
  return result
}

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

/** Energy in Wh, auto-scaled to MWh / kWh / Wh / mWh. */
function formatEnergy(wattHours: number): string {
  const absolute = Math.abs(wattHours)
  if (absolute >= 1e6) return `${stripTrailingZero((wattHours / 1e6).toFixed(2))} MWh`
  if (absolute >= 1e3) return `${stripTrailingZero((wattHours / 1e3).toFixed(2))} kWh`
  if (absolute >= 1) return `${stripTrailingZero(wattHours.toFixed(2))} Wh`
  if (absolute >= 1e-3)
    return `${stripTrailingZero((wattHours * 1000).toFixed(2))} mWh`
  return `${stripTrailingZero((wattHours * 1_000_000).toFixed(2))} µWh`
}

/** CO₂ in grams, auto-scaled to t / kg / g / mg. */
function formatCo2(grams: number): string {
  const absolute = Math.abs(grams)
  if (absolute >= 1_000_000)
    return `${stripTrailingZero((grams / 1_000_000).toFixed(2))} t CO₂e`
  if (absolute >= 1000)
    return `${stripTrailingZero((grams / 1000).toFixed(2))} kg CO₂e`
  if (absolute >= 1) return `${stripTrailingZero(grams.toFixed(2))} g CO₂e`
  if (absolute >= 1e-3)
    return `${stripTrailingZero((grams * 1000).toFixed(2))} mg CO₂e`
  return `${stripTrailingZero(grams.toFixed(4))} g CO₂e`
}

/** Water in liters, auto-scaled to kL / L / mL. */
function formatWater(liters: number): string {
  const absolute = Math.abs(liters)
  if (absolute >= 1000) return `${stripTrailingZero((liters / 1000).toFixed(2))} kL`
  if (absolute >= 1) return `${stripTrailingZero(liters.toFixed(2))} L`
  return `${stripTrailingZero((liters * 1000).toFixed(2))} mL`
}

/** Relative time — "12s ago", "3m ago", "4h ago", "2d ago". `null` returns
 *  "never". Used by the freshness chip in the environmental banner. */
function formatRelativeTime(isoTimestamp: string | null): string {
  if (isoTimestamp === null) return 'never'
  const eventMs = Date.parse(isoTimestamp)
  if (Number.isNaN(eventMs)) return 'unknown'
  const elapsedMs = Date.now() - eventMs
  if (elapsedMs < 0) return 'just now'
  const seconds = Math.floor(elapsedMs / 1000)
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  return `${days}d ago`
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
  // Page-level view switcher: dashboard (the default) vs the
  // methodology / transparency page. State-based rather than
  // URL-routed to keep the dependency surface minimal; if/when we
  // grow past two pages we can add react-router. Browser back button
  // doesn't preserve the methodology view across reloads — that's
  // an acceptable v0.1 trade.
  const [currentView, setCurrentView] = useState<'dashboard' | 'methodology'>('dashboard')

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
  const [billingChargesState, setBillingChargesState] = useState<FetchState<BillingChargesResponse>>({
    status: 'idle',
  })
  const [activeFactorsState, setActiveFactorsState] = useState<FetchState<ActiveFactorsResponse>>({
    status: 'idle',
  })

  // Bumped after a subscription / billing-import mutation so the GET re-runs.
  const [subscriptionsRevision, setSubscriptionsRevision] = useState(0)
  const refreshSubscriptions = () => setSubscriptionsRevision((current) => current + 1)
  const [billingChargesRevision, setBillingChargesRevision] = useState(0)
  const refreshBillingCharges = () => setBillingChargesRevision((current) => current + 1)

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

  // Active factor rows — fetched once on mount. The endpoint is
  // backed by the in-memory factor file snapshot, which only changes
  // when `tokenscale serve` is restarted with a new file. Stale-after-
  // upgrade is acceptable; users restart the server to update factors.
  useEffect(() => {
    let cancelled = false
    setActiveFactorsState({ status: 'loading' })
    fetch('/api/v1/factors/active')
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as ActiveFactorsResponse
      })
      .then((data) => {
        if (!cancelled) setActiveFactorsState({ status: 'ok', data })
      })
      .catch((error) => {
        if (!cancelled) {
          setActiveFactorsState({ status: 'error', message: (error as Error).message })
        }
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Billing charges — fetched once on mount and after any import
  // commit. Intentionally NOT scoped by the chart's date window:
  // the Subscriptions panel below is supposed to show "what
  // subscriptions exist", which is window-agnostic. Stat-row totals
  // filter this same data client-side for window-scoped sums.
  useEffect(() => {
    let cancelled = false
    setBillingChargesState({ status: 'loading' })
    fetch('/api/v1/billing/charges')
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return (await response.json()) as BillingChargesResponse
      })
      .then((data) => {
        if (!cancelled) setBillingChargesState({ status: 'ok', data })
      })
      .catch((error) => {
        if (!cancelled) {
          setBillingChargesState({ status: 'error', message: (error as Error).message })
        }
      })
    return () => {
      cancelled = true
    }
  }, [billingChargesRevision])

  // Health — once on mount, then every 30s so the freshness chip
  // ticks and the daily-data fetch below picks up new events that
  // the server's auto-scan task ingested in the background.
  useEffect(() => {
    let cancelled = false
    const fetchHealth = async () => {
      try {
        const response = await fetch('/api/v1/health')
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const data = (await response.json()) as HealthResponse
        if (!cancelled) setHealthState({ status: 'ok', data })
      } catch (error) {
        if (!cancelled) {
          setHealthState({ status: 'error', message: (error as Error).message })
        }
      }
    }
    void fetchHealth()
    const intervalId = window.setInterval(() => void fetchHealth(), 30_000)
    return () => {
      cancelled = true
      window.clearInterval(intervalId)
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

  // `last_scanned_at` from the most recent /health poll. Adding it to
  // the daily-fetch dependency list re-fires the chart query whenever
  // the server ingests new data — without it the user would have to
  // manually reload to see fresh events.
  const lastScannedAt =
    healthState.status === 'ok' ? healthState.data.ingest.last_scanned_at : null

  // Daily — refetched when SQL filters change OR when fresh data arrives.
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
  }, [
    providerFilter,
    selectedProjects,
    fromDate,
    toDate,
    effectiveGranularity,
    lastScannedAt,
  ])

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

    // Zero-fill across the requested window. The server only returns
    // buckets that actually have data, so a 1y view of a recently-started
    // dataset would otherwise collapse to whatever 21 days have data —
    // visually identical to a 30-day view, which the user reasonably
    // flagged as a bug. Enumerating every bucket in the window and merging
    // server data into it gives the chart the temporal context it needs.
    const allBucketDates = enumerateBuckets(fromDate, toDate, data.granularity)
    const dataByBucket = new Map(data.rows.map((row) => [row.date, row]))

    if (stackBy === 'model') {
      const series = visibleModels.map((modelId, index) => ({
        key: modelId,
        displayName: modelDisplayName(modelId),
        color: CHART_COLORS[index % CHART_COLORS.length],
      }))
      const rows = allBucketDates.map((bucketDate) => {
        const row = dataByBucket.get(bucketDate)
        const cells: Record<string, string | number> = { date: bucketDate }
        for (const modelId of visibleModels) {
          const tokens = row?.byModel[modelId]
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
    const rows = allBucketDates.map((bucketDate) => {
      const row = dataByBucket.get(bucketDate)
      const cells: Record<string, string | number> = { date: bucketDate }
      for (const tokenType of tokenTypeKeys) {
        let sum = 0
        if (row) {
          for (const modelId of visibleModels) {
            const tokens = row.byModel[modelId]
            if (!tokens) continue
            sum += tokenFieldsForView(tokens, viewMode, data.pricingByModel[modelId])[
              tokenType as keyof BillableBreakdown
            ]
          }
        }
        cells[tokenType] = sum
      }
      return cells
    })
    return { rows, series, hiddenInPricedView }
  }, [dailyState, selectedModels, selectedTokenTypes, stackBy, viewMode, fromDate, toDate])

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

  // Imported billing charges split by category AND filtered to the
  // current chart window. (The underlying fetch is window-agnostic so
  // the Subscriptions panel below can list all history; the stat row
  // here re-filters by date for "what was paid in this window".)
  // Subscription-category rows belong to the "Subscriptions paid"
  // stat; everything else (overage / one-time / refund / unknown)
  // belongs in "Other charges". Refunds are negative-amount rows so
  // they reduce their respective bucket — that's the right accounting.
  const importedSubscriptionsUsd = useMemo(() => {
    if (billingChargesState.status !== 'ok') return null
    return billingChargesState.data.charges
      .filter(
        (charge) =>
          charge.category === 'subscription' &&
          charge.occurred_at >= fromDate &&
          charge.occurred_at <= toDate,
      )
      .reduce((sum, charge) => sum + charge.amount_usd, 0)
  }, [billingChargesState, fromDate, toDate])
  const importedOtherUsd = useMemo(() => {
    if (billingChargesState.status !== 'ok') return null
    return billingChargesState.data.charges
      .filter(
        (charge) =>
          charge.category !== 'subscription' &&
          charge.occurred_at >= fromDate &&
          charge.occurred_at <= toDate,
      )
      .reduce((sum, charge) => sum + charge.amount_usd, 0)
  }, [billingChargesState, fromDate, toDate])

  // "Subscriptions paid in window" combines manually-declared
  // subscriptions (pro-rated over their date overlap) with imported
  // subscription-category charges (already date-stamped — included if
  // their occurred_at falls in window). Dedup is handled at import
  // time: dismissing a manual entry during the preview removes it
  // from `subscriptions`, so summing is safe.
  const combinedSubscriptionsUsd =
    subscriptionCostUsd === null && importedSubscriptionsUsd === null
      ? null
      : (subscriptionCostUsd ?? 0) + (importedSubscriptionsUsd ?? 0)

  // "Other charges in window" is overages + one-time + refunds. The
  // CSV import surfaces these distinctly from subscriptions so the
  // dashboard can split them in the stat row.
  const otherChargesUsd = importedOtherUsd

  const totalBilledUsd =
    combinedSubscriptionsUsd === null && otherChargesUsd === null
      ? null
      : (combinedSubscriptionsUsd ?? 0) + (otherChargesUsd ?? 0)

  const netValueUsd =
    counterfactualCostUsd !== null && totalBilledUsd !== null
      ? counterfactualCostUsd - totalBilledUsd
      : null

  // Window-total environmental impact, summed across the chart's currently
  // visible (model) cells. Token-type filter is intentionally NOT applied
  // — impact is computed at the per-event factor level upstream of the
  // bucket, so chopping by token type after the fact would mis-attribute
  // facility overhead. Models are filtered to honor the chip selection.
  const windowImpact = useMemo(() => {
    if (dailyState.status !== 'ok') return null
    const data = dailyState.data
    const visibleModels = data.models.filter((m) => isSelected(selectedModels, m))
    let energyWh = 0
    let facilityWh = 0
    let co2eG = 0
    let waterL = 0
    let energyUncertaintyPct = 0
    let co2eUncertaintyPct = 0
    let waterUncertaintyPct = 0
    let eventsMissingFactor = 0
    let eventsCount = 0
    let anyCo2 = false
    let anyWater = false
    for (const row of data.rows) {
      for (const modelId of visibleModels) {
        const cell = row.byModel[modelId]
        if (!cell) continue
        const impact = cell.impact
        energyWh += impact.energy_wh
        facilityWh += impact.facility_wh
        if (impact.co2eG !== null) {
          co2eG += impact.co2eG
          anyCo2 = true
        }
        if (impact.waterL !== null) {
          waterL += impact.waterL
          anyWater = true
        }
        if (impact.maxUncertaintyPct > energyUncertaintyPct) {
          energyUncertaintyPct = impact.maxUncertaintyPct
        }
        if (impact.co2eUncertaintyPct > co2eUncertaintyPct) {
          co2eUncertaintyPct = impact.co2eUncertaintyPct
        }
        if (impact.waterUncertaintyPct > waterUncertaintyPct) {
          waterUncertaintyPct = impact.waterUncertaintyPct
        }
        eventsMissingFactor += impact.eventsMissingEnvFactor
        eventsCount += impact.eventsCount
      }
    }
    return {
      energyWh,
      facilityWh,
      co2eG: anyCo2 ? co2eG : null,
      waterL: anyWater ? waterL : null,
      energyUncertaintyPct,
      co2eUncertaintyPct,
      waterUncertaintyPct,
      eventsMissingFactor,
      eventsCount,
    }
  }, [dailyState, selectedModels])

  const allModels = dailyState.status === 'ok' ? dailyState.data.models : []
  const allTokenTypes = dailyState.status === 'ok' ? dailyState.data.tokenTypes : []
  const allProjectIds = projectsState.status === 'ok'
    ? projectsState.data.projects.map((p) => p.project_id)
    : []
  const pricingNeedsReview =
    healthState.status === 'ok' && healthState.data.pricing.needs_review
  const pricingAccessedAt =
    healthState.status === 'ok' ? healthState.data.pricing.accessed_at : null
  const environmentalHealth =
    healthState.status === 'ok' ? healthState.data.environmental : null
  // Environmental views are meaningful only when the factor file is
  // production-grade. Phase 1's placeholder file flips this off so the
  // banner doesn't promise data that isn't there.
  const environmentalReady =
    environmentalHealth !== null &&
    environmentalHealth.file_status === 'production' &&
    !environmentalHealth.is_placeholder

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
          <nav className="flex items-baseline gap-1 text-sm">
            <button
              type="button"
              onClick={() => setCurrentView('dashboard')}
              className={
                'px-3 py-1 rounded-md transition-colors ' +
                (currentView === 'dashboard'
                  ? 'bg-blue-50 text-blue-700 font-medium'
                  : 'text-slate-600 hover:bg-slate-50')
              }
            >
              Dashboard
            </button>
            <button
              type="button"
              onClick={() => setCurrentView('methodology')}
              className={
                'px-3 py-1 rounded-md transition-colors ' +
                (currentView === 'methodology'
                  ? 'bg-blue-50 text-blue-700 font-medium'
                  : 'text-slate-600 hover:bg-slate-50')
              }
            >
              Methodology
            </button>
          </nav>
        </div>
      </header>

      {environmentalReady && environmentalHealth && (
        <div className="border-b border-emerald-200 bg-emerald-50 text-emerald-900 text-xs px-6 py-2">
          <span className="font-medium">
            Environmental factors v{environmentalHealth.file_version ?? '?'}
            {environmentalHealth.file_published
              ? ` · ${environmentalHealth.file_published}`
              : ''}
          </span>
          {' · '}
          Region:{' '}
          <span className="font-mono">{environmentalHealth.configured_region}</span>
          {environmentalHealth.configured_region_egrid_subregion && (
            <>
              {' → eGRID '}
              <span className="font-mono">
                {environmentalHealth.configured_region_egrid_subregion}
              </span>
              {environmentalHealth.configured_region_egrid_subregion_full_name && (
                <>
                  {' ('}
                  {environmentalHealth.configured_region_egrid_subregion_full_name}
                  {')'}
                </>
              )}
            </>
          )}
          {environmentalHealth.methodology_source && (
            <>
              {' · '}
              <a
                className="underline"
                href={environmentalHealth.methodology_source}
                target="_blank"
                rel="noreferrer"
              >
                {environmentalHealth.methodology ?? 'methodology'}
              </a>
            </>
          )}
          {' · '}
          <span title="Most recent file scan. The server runs an incremental scan in the background — set [ingest].scan_interval_seconds in your config.">
            Last scanned: {formatRelativeTime(lastScannedAt)}
          </span>
        </div>
      )}

      {currentView === 'dashboard' &&
        (viewMode === 'billable' || viewMode === 'cost') &&
        pricingAccessedAt && (
        <div
          className={
            'border-b text-xs px-6 py-2 ' +
            (pricingNeedsReview
              ? 'bg-amber-50 border-amber-200 text-amber-900'
              : 'bg-slate-50 border-slate-200 text-slate-600')
          }
        >
          {pricingNeedsReview ? (
            <>
              <span className="font-medium">Pricing as of {pricingAccessedAt}</span> — published
              Anthropic list rates as recorded in{' '}
              <code className="bg-amber-100 px-1 rounded">pricing.toml</code>. The maintainer
              hasn't re-verified for this build, so re-check against{' '}
              <a
                className="underline"
                href="https://platform.claude.com/docs/en/about-claude/pricing"
                target="_blank"
                rel="noreferrer"
              >
                current Anthropic prices
              </a>{' '}
              if rates may have changed since then.
            </>
          ) : (
            <>
              Pricing reflects published Anthropic list rates as of{' '}
              <span className="font-medium">{pricingAccessedAt}</span> (
              <a
                className="underline"
                href="https://platform.claude.com/docs/en/about-claude/pricing"
                target="_blank"
                rel="noreferrer"
              >
                source
              </a>
              ).
            </>
          )}
        </div>
      )}

      {currentView === 'methodology' && <MethodologyPage />}
      {currentView === 'dashboard' && (
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
            combinedSubscriptionsUsd={combinedSubscriptionsUsd}
            otherChargesUsd={otherChargesUsd}
            totalBilledUsd={totalBilledUsd}
            netValueUsd={netValueUsd}
            hasSubscriptionsData={
              (subscriptionsState.status === 'ok' &&
                subscriptionsState.data.subscriptions.length > 0) ||
              (importedSubscriptionsUsd !== null && importedSubscriptionsUsd !== 0)
            }
            hasOtherCharges={
              billingChargesState.status === 'ok' &&
              billingChargesState.data.charges.some(
                (charge) => charge.category !== 'subscription',
              )
            }
            pricingNeedsReview={pricingNeedsReview}
          />

          {environmentalReady && windowImpact && (
            <EnvironmentalStatRow
              impact={windowImpact}
              modelsWithoutFactors={
                dailyState.status === 'ok' ? dailyState.data.modelsWithoutFactors : []
              }
            />
          )}

          {environmentalReady && activeFactorsState.status === 'ok' && (
            <FactorProvenancePanel
              activeFactors={activeFactorsState.data}
              visibleModelIds={
                dailyState.status === 'ok' ? dailyState.data.models : []
              }
            />
          )}

          <div>
            <h2 className="text-base font-medium mb-3">
              {/* Bucket-cadence label mirrors the actual granularity
                  (auto-picked or user-overridden). Without this, a wide
                  window auto-picks weekly buckets but the title still
                  says "Daily" — the y-axis peaks suddenly look ~7×
                  larger than the user expects and there's no on-screen
                  cue why. */}
              {chartTitleForGranularity(effectiveGranularity, viewMode)}{' '}
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
            <p
              className="text-xs text-slate-500 mt-3"
              title="Anthropic does not currently expose a per-user usage feed for consumer products. This is true of ChatGPT, Gemini, and Mistral as well. See docs/data-sources.md."
            >
              Chart shows Claude Code session data only. Usage from the Claude
              iOS / Android / desktop apps and{' '}
              <span className="font-mono">claude.ai</span> is structurally invisible —
              Anthropic doesn't expose a feed for consumer products. The COST of
              those products IS tracked, via your imported subscription charges.
            </p>
          </div>
        </section>

        <SubscriptionsPanel
          subscriptionsState={subscriptionsState}
          billingChargesState={billingChargesState}
          onMutated={refreshSubscriptions}
        />

        {/*
         * Manually-composed CSV import is the user's chosen path for
         * tracking Anthropic billing — neither claude.ai nor the
         * Console exposes a bulk export, and the Admin API requires
         * an organization account (not available on individual tier).
         * The panel is therefore the practical entry point until one
         * of those changes; auto-ingest from the Admin API will reuse
         * the same backend (`billing_charges` table,
         * `insert_billing_charges` path) under
         * `source = "anthropic_admin"` when that path opens up.
         */}
        <BillingImportPanel
          billingChargesState={billingChargesState}
          onCommitted={() => {
            refreshSubscriptions()
            refreshBillingCharges()
          }}
        />
      </main>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// StatRow — counterfactual API cost, combined subscriptions, other
// charges, net value. Subscriptions = manual entries + imported rows
// tagged `subscription` (one bucket, one card). Other charges =
// imported rows tagged anything else (overages, one-time, refunds).
// ---------------------------------------------------------------------------

type StatRowProps = {
  counterfactualCostUsd: number | null
  combinedSubscriptionsUsd: number | null
  otherChargesUsd: number | null
  totalBilledUsd: number | null
  netValueUsd: number | null
  hasSubscriptionsData: boolean
  hasOtherCharges: boolean
  pricingNeedsReview: boolean
}

function StatRow({
  counterfactualCostUsd,
  combinedSubscriptionsUsd,
  otherChargesUsd,
  totalBilledUsd,
  netValueUsd,
  hasSubscriptionsData,
  hasOtherCharges,
  pricingNeedsReview,
}: StatRowProps) {
  if (counterfactualCostUsd === null) return null

  const counterfactualLabel = pricingNeedsReview
    ? 'Counterfactual API cost (approx)'
    : 'Counterfactual API cost'

  // The "Other charges" card only appears when there's something to
  // show in it — pre-import, the user has just subscriptions, and a
  // 4-card row with a permanently-muted card adds visual noise.
  const showOtherChargesCard = hasOtherCharges

  return (
    <div
      className={
        showOtherChargesCard
          ? 'grid grid-cols-1 sm:grid-cols-4 gap-3'
          : 'grid grid-cols-1 sm:grid-cols-3 gap-3'
      }
    >
      <StatCard
        label={counterfactualLabel}
        value={formatExactDollars(counterfactualCostUsd)}
        helpText="What these tokens would have cost on the Anthropic API at list rates, summed across the chart's currently visible cells."
      />
      <StatCard
        label="Subscriptions paid in window"
        value={
          combinedSubscriptionsUsd === null
            ? '—'
            : formatExactDollars(combinedSubscriptionsUsd)
        }
        helpText={
          hasSubscriptionsData
            ? 'Manually-declared subscriptions (pro-rated over each window overlap) PLUS imported billing rows tagged "subscription" whose date falls in window. The import preview keeps these from double-counting against manual entries.'
            : 'No subscriptions declared yet. Add one below or import a billing CSV to populate this automatically.'
        }
        muted={!hasSubscriptionsData}
      />
      {showOtherChargesCard && (
        <StatCard
          label="Other charges in window"
          value={
            otherChargesUsd === null
              ? '—'
              : formatExactDollars(otherChargesUsd)
          }
          helpText="Imported billing rows tagged overage / one-time / refund — anything that's not a recurring subscription. Refunds are negative-amount rows that reduce the total."
          muted={!hasOtherCharges}
        />
      )}
      <StatCard
        label="Net value"
        value={
          netValueUsd === null
            ? '—'
            : formatExactDollars(netValueUsd)
        }
        helpText={
          showOtherChargesCard
            ? `Counterfactual API cost minus everything you've actually paid (subscriptions + other charges${totalBilledUsd === null ? '' : ` = ${formatExactDollars(totalBilledUsd)}`}). Positive means your plan is cheaper than running the same usage on the API.`
            : 'Counterfactual API cost minus subscriptions paid. Positive means your subscription is cheaper than running the same usage on the API.'
        }
        emphasize={netValueUsd !== null && netValueUsd > 0}
      />
    </div>
  )
}

// ---------------------------------------------------------------------------
// EnvironmentalStatRow — three KPIs: total energy, CO₂e, water for the
// chart's currently visible cells, with the bucket-max uncertainty
// percentage applied to all three. Renders only when the factor file is
// production-grade; "factor data unavailable" is communicated via the
// `modelsWithoutFactors` footnote.
// ---------------------------------------------------------------------------

type EnvironmentalStatRowProps = {
  impact: {
    energyWh: number
    facilityWh: number
    co2eG: number | null
    waterL: number | null
    energyUncertaintyPct: number
    co2eUncertaintyPct: number
    waterUncertaintyPct: number
    eventsMissingFactor: number
    eventsCount: number
  }
  modelsWithoutFactors: string[]
}

function EnvironmentalStatRow({
  impact,
  modelsWithoutFactors,
}: EnvironmentalStatRowProps) {
  const suffix = (pct: number) => (pct > 0 ? ` ± ${pct}%` : '')
  const energyValue = `${formatEnergy(impact.facilityWh)}${suffix(impact.energyUncertaintyPct)}`
  const co2eValue =
    impact.co2eG === null
      ? '—'
      : `${formatCo2(impact.co2eG)}${suffix(impact.co2eUncertaintyPct)}`
  const waterValue =
    impact.waterL === null
      ? '—'
      : `${formatWater(impact.waterL)}${suffix(impact.waterUncertaintyPct)}`

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <StatCard
          label="Energy (facility)"
          value={energyValue}
          helpText="Per-event energy summed across visible cells, including PUE-weighted facility overhead. ± band is the widest model uncertainty in the window."
        />
        <StatCard
          label="CO₂e"
          value={co2eValue}
          helpText="Greenhouse-gas emissions attributed via the configured region's grid intensity. ± band combines model and grid CO₂e uncertainty via quadrature. Anthropic does not disclose which region served any given request — region is your declared assumption."
          muted={impact.co2eG === null}
        />
        <StatCard
          label="Water"
          value={waterValue}
          helpText="Direct datacenter water (cooling) using the configured region's WUE, falling back to the file's defaults.fallback_wue_l_per_kwh when absent. ± band combines model and grid water uncertainty via quadrature. Indirect (power-plant) water is a future enhancement."
          muted={impact.waterL === null}
        />
      </div>
      {modelsWithoutFactors.length > 0 && (
        <div className="text-xs text-slate-500">
          Factor data unavailable:{' '}
          {modelsWithoutFactors.map((m) => modelDisplayName(m)).join(', ')}
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// FactorProvenancePanel — surfaces which env_factors / grid_factors rows
// the dashboard's environmental numbers currently rest on. Collapsed by
// default; expanding shows the per-(provider, model) row metadata plus
// the configured region's grid row, with links back to the methodology
// page and the bibliography. v0.1 of per-value provenance — v0.2
// would tie it to specific chart cells on hover.
// ---------------------------------------------------------------------------

type FactorProvenancePanelProps = {
  activeFactors: ActiveFactorsResponse
  /// Models present in the current chart window. Used to filter the
  /// "Models" list to just what the user is actually seeing —
  /// surfacing the full 17-row factor file would be noise.
  visibleModelIds: string[]
}

function FactorProvenancePanel({
  activeFactors,
  visibleModelIds,
}: FactorProvenancePanelProps) {
  const [expanded, setExpanded] = useState(false)

  const visibleModelSet = new Set(visibleModelIds)
  const visibleModels = activeFactors.models.filter((model) =>
    visibleModelSet.has(model.model_id),
  )
  const configuredRegion = activeFactors.regions.find(
    (region) => region.region_id === activeFactors.configured_region,
  )

  if (visibleModels.length === 0 && !configuredRegion) {
    return null
  }

  return (
    <section className="rounded-md border border-slate-200 bg-slate-50">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center justify-between px-4 py-2 text-sm text-slate-700 hover:bg-slate-100 rounded-md"
      >
        <span className="flex items-center gap-2">
          <Chevron direction={expanded ? 'down' : 'right'} />
          <span className="font-medium">Sources for these numbers</span>
          <span className="text-xs text-slate-500">
            ({visibleModels.length} model{visibleModels.length === 1 ? '' : 's'}
            {configuredRegion ? ' · 1 region' : ''}
            {activeFactors.methodology ? ' · methodology' : ''})
          </span>
        </span>
        <span className="text-xs text-slate-500">
          {expanded ? 'Hide' : 'Show'}
        </span>
      </button>

      {expanded && (
        <div className="px-4 py-3 border-t border-slate-200 space-y-4 text-sm">
          {/* Methodology link */}
          {activeFactors.methodology && (
            <div className="space-y-1">
              <div className="text-xs font-medium text-slate-500 uppercase tracking-wide">
                Methodology
              </div>
              <div className="text-slate-700">
                <span className="font-mono">{activeFactors.methodology}</span>
                {activeFactors.methodology_source && (
                  <>
                    {' · '}
                    <a
                      className="underline text-blue-600 hover:text-blue-700"
                      href={activeFactors.methodology_source}
                      target="_blank"
                      rel="noreferrer"
                    >
                      source paper
                    </a>
                  </>
                )}
                {activeFactors.file_version && (
                  <span className="text-slate-500">
                    {' · '}factor file v{activeFactors.file_version}
                  </span>
                )}
              </div>
            </div>
          )}

          {/* Per-model factor rows */}
          {visibleModels.length > 0 && (
            <div className="space-y-2">
              <div className="text-xs font-medium text-slate-500 uppercase tracking-wide">
                Model factors (per-token energy)
              </div>
              <ul className="space-y-2">
                {visibleModels.map((model) => (
                  <li
                    key={`${model.provider}/${model.model_id}`}
                    className="rounded border border-slate-200 bg-white px-3 py-2"
                  >
                    <div className="flex items-baseline justify-between gap-3">
                      <span className="font-medium text-slate-900">
                        {model.display_name}
                      </span>
                      <span className="text-xs text-slate-500">
                        {model.confidence && (
                          <span className="font-mono mr-2">{model.confidence}</span>
                        )}
                        {model.uncertainty_range_pct !== null &&
                          model.uncertainty_range_pct !== 0 &&
                          `± ${model.uncertainty_range_pct}%`}
                      </span>
                    </div>
                    <div className="text-xs text-slate-500 mt-1 space-x-3">
                      {model.valid_from && <span>valid from {model.valid_from}</span>}
                      {model.source_doc && (
                        <span>
                          src:{' '}
                          <span className="font-mono">{model.source_doc}</span>
                        </span>
                      )}
                    </div>
                    {model.notes && (
                      <details className="text-xs text-slate-600 mt-1">
                        <summary className="cursor-pointer text-slate-500">
                          Notes
                        </summary>
                        <div className="mt-1 pl-3 border-l-2 border-slate-200">
                          {model.notes}
                        </div>
                      </details>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* Configured region */}
          {configuredRegion && (
            <div className="space-y-2">
              <div className="text-xs font-medium text-slate-500 uppercase tracking-wide">
                Grid factors (CO₂e + water + PUE)
              </div>
              <div className="rounded border border-slate-200 bg-white px-3 py-2">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="font-medium text-slate-900">
                    {configuredRegion.display_name}
                  </span>
                  <span className="text-xs text-slate-500 font-mono">
                    {configuredRegion.region_id}
                  </span>
                </div>
                <div className="text-xs text-slate-500 mt-1 space-y-0.5">
                  {configuredRegion.egrid_subregion && (
                    <div>
                      eGRID:{' '}
                      <span className="font-mono">
                        {configuredRegion.egrid_subregion}
                      </span>
                      {configuredRegion.egrid_subregion_full_name && (
                        <span>
                          {' '}({configuredRegion.egrid_subregion_full_name})
                        </span>
                      )}
                    </div>
                  )}
                  <div>
                    {configuredRegion.co2e_kg_per_kwh !== null && (
                      <span className="mr-3">
                        CO₂e:{' '}
                        <span className="font-mono">
                          {configuredRegion.co2e_kg_per_kwh} kg/kWh
                        </span>
                        {configuredRegion.co2e_uncertainty_range_pct !== null && (
                          <span className="text-slate-400">
                            {' '}± {configuredRegion.co2e_uncertainty_range_pct}%
                          </span>
                        )}
                      </span>
                    )}
                    {configuredRegion.water_l_per_kwh !== null && (
                      <span className="mr-3">
                        Water:{' '}
                        <span className="font-mono">
                          {configuredRegion.water_l_per_kwh} L/kWh
                        </span>
                        {configuredRegion.water_uncertainty_range_pct !== null && (
                          <span className="text-slate-400">
                            {' '}± {configuredRegion.water_uncertainty_range_pct}%
                          </span>
                        )}
                      </span>
                    )}
                    {configuredRegion.pue !== null && (
                      <span>
                        PUE:{' '}
                        <span className="font-mono">{configuredRegion.pue}</span>
                      </span>
                    )}
                  </div>
                  {configuredRegion.source_url_co2e && (
                    <div>
                      <a
                        className="underline text-blue-600 hover:text-blue-700"
                        href={configuredRegion.source_url_co2e}
                        target="_blank"
                        rel="noreferrer"
                      >
                        EPA eGRID source
                      </a>
                    </div>
                  )}
                </div>
              </div>
            </div>
          )}

          <p className="text-xs text-slate-500 pt-1 border-t border-slate-200">
            Full narrative + bibliography on the{' '}
            <button
              type="button"
              // Note: button only, can't link to sibling state directly
              // — user navigates via the header tab.
              className="underline text-blue-600 hover:text-blue-700"
              onClick={() => {
                window.scrollTo({ top: 0, behavior: 'smooth' })
              }}
              title="Click 'Methodology' in the header"
            >
              Methodology
            </button>{' '}
            tab.
          </p>
        </div>
      )}
    </section>
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
// BillingImportPanel — drop a Stripe CSV in, preview the rows, dismiss
// any manual subscriptions that the CSV would double-count, commit.
// Two-step flow (preview/commit) so destructive writes are gated
// behind a "user reviewed" checkpoint without re-parsing on commit.
// ---------------------------------------------------------------------------

type ImportPanelState =
  | { kind: 'idle' }
  | { kind: 'previewing' }
  | { kind: 'previewed'; preview: BillingPreviewResponse; csvText: string }
  | { kind: 'committing'; preview: BillingPreviewResponse; csvText: string }
  | { kind: 'success'; summary: BillingCommitResponse }
  | { kind: 'error'; message: string }

type BillingImportPanelProps = {
  billingChargesState: FetchState<BillingChargesResponse>
  onCommitted: () => void
}

function BillingImportPanel({ billingChargesState, onCommitted }: BillingImportPanelProps) {
  const [csvText, setCsvText] = useState('')
  const [importState, setImportState] = useState<ImportPanelState>({ kind: 'idle' })
  // Per-row category overrides keyed by external_id (or row index for
  // rows without one). Set on dropdown change; merged into the
  // PreviewedCharge payload at commit time.
  const [categoryOverrides, setCategoryOverrides] = useState<Map<string, BillingCategory>>(
    new Map(),
  )
  // Manual subscriptions the user marked for dismissal. Stored as a
  // Set of subscription IDs; merged into the commit payload.
  const [dismissedSubscriptions, setDismissedSubscriptions] = useState<Set<number>>(new Set())
  const [collapsed, setCollapsed] = useState(true)

  function rowKey(charge: PreviewedCharge, index: number): string {
    return charge.external_id ?? `row:${index}`
  }

  async function runPreview() {
    if (csvText.trim().length === 0) {
      setImportState({ kind: 'error', message: 'Paste or upload CSV content first.' })
      return
    }
    setImportState({ kind: 'previewing' })
    setCategoryOverrides(new Map())
    setDismissedSubscriptions(new Set())
    try {
      const response = await fetch('/api/v1/billing/charges/preview', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ csv: csvText }),
      })
      if (!response.ok) {
        const text = await response.text()
        throw new Error(text || `HTTP ${response.status}`)
      }
      const preview = (await response.json()) as BillingPreviewResponse
      setImportState({ kind: 'previewed', preview, csvText })
    } catch (error) {
      setImportState({ kind: 'error', message: (error as Error).message })
    }
  }

  async function runCommit() {
    if (importState.kind !== 'previewed') return
    const previewState = importState
    setImportState({
      kind: 'committing',
      preview: previewState.preview,
      csvText: previewState.csvText,
    })
    const finalCharges: PreviewedCharge[] = previewState.preview.charges.map((charge, index) => ({
      ...charge,
      category: categoryOverrides.get(rowKey(charge, index)) ?? charge.category,
    }))
    try {
      const response = await fetch('/api/v1/billing/charges/commit', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          charges: finalCharges,
          dismiss_subscription_ids: Array.from(dismissedSubscriptions),
        }),
      })
      if (!response.ok) {
        const text = await response.text()
        throw new Error(text || `HTTP ${response.status}`)
      }
      const summary = (await response.json()) as BillingCommitResponse
      setImportState({ kind: 'success', summary })
      setCsvText('')
      setCategoryOverrides(new Map())
      setDismissedSubscriptions(new Set())
      onCommitted()
    } catch (error) {
      setImportState({ kind: 'error', message: (error as Error).message })
    }
  }

  function reset() {
    setImportState({ kind: 'idle' })
    setCsvText('')
    setCategoryOverrides(new Map())
    setDismissedSubscriptions(new Set())
  }

  async function handleFileInput(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0]
    if (!file) return
    const text = await file.text()
    setCsvText(text)
  }

  const importedCount =
    billingChargesState.status === 'ok' ? billingChargesState.data.charges.length : 0

  return (
    <section className="bg-white rounded-lg border border-slate-200 p-5 space-y-4">
      <div className="flex items-baseline justify-between gap-4">
        <div>
          <h2 className="text-base font-medium">Import billing CSV</h2>
          <p className="text-xs text-slate-500 mt-1">
            Drop a Stripe Customer Portal export to track subscriptions and overages
            automatically. Re-imports are idempotent — already-known charges are skipped.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCollapsed((c) => !c)}
          className="text-sm text-blue-600 hover:underline shrink-0"
        >
          {collapsed
            ? `Open${importedCount > 0 ? ` (${importedCount} imported)` : ''}`
            : 'Close'}
        </button>
      </div>

      {!collapsed && (
        <div className="space-y-4">
          {(importState.kind === 'idle' || importState.kind === 'previewing'
            || importState.kind === 'error') && (
            <>
              <div className="flex items-center gap-3 text-xs">
                <label className="cursor-pointer rounded-md border border-slate-300 px-3 py-1.5 hover:bg-slate-50">
                  Choose file
                  <input
                    type="file"
                    accept=".csv,text/csv"
                    className="hidden"
                    onChange={handleFileInput}
                  />
                </label>
                <span className="text-slate-400">or paste below</span>
              </div>
              <textarea
                rows={8}
                value={csvText}
                onChange={(event) => setCsvText(event.target.value)}
                placeholder="id,Date,Description,Amount,Currency&#10;in_xxx,2026-04-15,Claude Max Subscription,200.00,USD&#10;..."
                className="w-full font-mono text-xs border border-slate-300 rounded-md px-3 py-2 bg-white"
              />
              {importState.kind === 'error' && (
                <div className="text-xs text-red-700 bg-red-50 border border-red-200 rounded-md px-3 py-2">
                  {importState.message}
                </div>
              )}
              <button
                type="button"
                disabled={importState.kind === 'previewing'}
                onClick={runPreview}
                className="rounded-md bg-blue-600 text-white text-sm px-4 py-1.5 hover:bg-blue-700 disabled:opacity-50"
              >
                {importState.kind === 'previewing' ? 'Parsing…' : 'Preview'}
              </button>
            </>
          )}

          {(importState.kind === 'previewed' || importState.kind === 'committing') && (
            <BillingImportPreview
              preview={importState.preview}
              categoryOverrides={categoryOverrides}
              setCategoryOverrides={setCategoryOverrides}
              dismissedSubscriptions={dismissedSubscriptions}
              setDismissedSubscriptions={setDismissedSubscriptions}
              committing={importState.kind === 'committing'}
              onCommit={runCommit}
              onCancel={reset}
              rowKey={rowKey}
            />
          )}

          {importState.kind === 'success' && (
            <div className="space-y-3">
              <div className="text-xs text-emerald-800 bg-emerald-50 border border-emerald-200 rounded-md px-3 py-2">
                Imported {importState.summary.inserted} new charge
                {importState.summary.inserted === 1 ? '' : 's'} ·{' '}
                skipped {importState.summary.skipped_duplicate} duplicate
                {importState.summary.skipped_duplicate === 1 ? '' : 's'} ·{' '}
                dismissed {importState.summary.dismissed_subscriptions} manual subscription
                {importState.summary.dismissed_subscriptions === 1 ? '' : 's'}.
              </div>
              <button
                type="button"
                onClick={reset}
                className="text-sm text-blue-600 hover:underline"
              >
                Import another
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  )
}

type BillingImportPreviewProps = {
  preview: BillingPreviewResponse
  categoryOverrides: Map<string, BillingCategory>
  setCategoryOverrides: React.Dispatch<React.SetStateAction<Map<string, BillingCategory>>>
  dismissedSubscriptions: Set<number>
  setDismissedSubscriptions: React.Dispatch<React.SetStateAction<Set<number>>>
  committing: boolean
  onCommit: () => void
  onCancel: () => void
  rowKey: (charge: PreviewedCharge, index: number) => string
}

function BillingImportPreview({
  preview,
  categoryOverrides,
  setCategoryOverrides,
  dismissedSubscriptions,
  setDismissedSubscriptions,
  committing,
  onCommit,
  onCancel,
  rowKey,
}: BillingImportPreviewProps) {
  function setOverride(key: string, category: BillingCategory) {
    setCategoryOverrides((current) => {
      const next = new Map(current)
      next.set(key, category)
      return next
    })
  }

  function toggleDismiss(id: number) {
    setDismissedSubscriptions((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  return (
    <div className="space-y-4">
      <div className="text-xs text-slate-600">
        Parsed <span className="font-medium">{preview.charges.length}</span> charge
        {preview.charges.length === 1 ? '' : 's'} totaling{' '}
        <span className="font-medium">{formatExactDollars(preview.total_amount_usd)}</span>
        {preview.skipped_non_usd_count > 0 && (
          <>
            {' '}
            · <span className="text-amber-700">
              skipped {preview.skipped_non_usd_count} non-USD line
              {preview.skipped_non_usd_count === 1 ? '' : 's'}
            </span>
          </>
        )}
        .
      </div>

      {preview.conflicting_subscriptions.length > 0 && (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-3 space-y-2">
          <div className="text-xs font-medium text-amber-900">
            Conflicts with manually-declared subscriptions
          </div>
          <p className="text-xs text-amber-800">
            These manual subscriptions overlap with subscription charges in the CSV. Dismiss
            them to avoid double-counting; they'll be deleted when you click Import below.
          </p>
          <ul className="space-y-1.5">
            {preview.conflicting_subscriptions.map((conflict) => {
              const dismissed = dismissedSubscriptions.has(conflict.id)
              return (
                <li key={conflict.id} className="flex items-start gap-2 text-xs">
                  <input
                    type="checkbox"
                    checked={dismissed}
                    onChange={() => toggleDismiss(conflict.id)}
                    className="mt-0.5"
                  />
                  <div>
                    <div className={dismissed ? 'line-through text-amber-700' : 'text-amber-900'}>
                      <span className="font-medium">{conflict.plan_name}</span> ·{' '}
                      {formatExactDollars(conflict.monthly_usd)}/mo · {conflict.started_at} →{' '}
                      {conflict.ended_at ?? 'open'}
                    </div>
                    <div className="text-amber-700 text-[11px]">
                      Overlaps {conflict.overlapping_charge_external_ids.length} CSV row
                      {conflict.overlapping_charge_external_ids.length === 1 ? '' : 's'}
                    </div>
                  </div>
                </li>
              )
            })}
          </ul>
        </div>
      )}

      <div className="overflow-x-auto">
        <table className="min-w-full text-xs border border-slate-200 rounded-md">
          <thead className="bg-slate-50 text-slate-600">
            <tr>
              <th className="text-left font-medium px-3 py-2">Date</th>
              <th className="text-left font-medium px-3 py-2">Description</th>
              <th className="text-right font-medium px-3 py-2">Amount</th>
              <th className="text-left font-medium px-3 py-2">Category</th>
              <th className="text-left font-medium px-3 py-2">ID</th>
            </tr>
          </thead>
          <tbody>
            {preview.charges.map((charge, index) => {
              const key = rowKey(charge, index)
              const effectiveCategory = categoryOverrides.get(key) ?? charge.category
              return (
                <tr key={key} className="border-t border-slate-200">
                  <td className="px-3 py-2 font-mono">{charge.occurred_at}</td>
                  <td className="px-3 py-2">{charge.description || '—'}</td>
                  <td className="px-3 py-2 text-right tabular-nums">
                    {formatExactDollars(charge.amount_usd)}
                  </td>
                  <td className="px-3 py-2">
                    <select
                      value={effectiveCategory}
                      onChange={(event) =>
                        setOverride(key, event.target.value as BillingCategory)
                      }
                      className="border border-slate-300 rounded px-2 py-1 bg-white"
                    >
                      {BILLING_CATEGORY_OPTIONS.map((option) => (
                        <option key={option.value} value={option.value}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td className="px-3 py-2 font-mono text-slate-500">
                    {charge.external_id ?? '—'}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <div className="flex items-center gap-3">
        <button
          type="button"
          disabled={committing}
          onClick={onCommit}
          className="rounded-md bg-blue-600 text-white text-sm px-4 py-1.5 hover:bg-blue-700 disabled:opacity-50"
        >
          {committing
            ? 'Importing…'
            : `Import ${preview.charges.length} charge${preview.charges.length === 1 ? '' : 's'}`}
        </button>
        <button
          type="button"
          disabled={committing}
          onClick={onCancel}
          className="text-sm text-slate-600 hover:underline disabled:opacity-50"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// SubscriptionsPanel — list, add (inline form), delete.
// ---------------------------------------------------------------------------

type FormMode =
  | { kind: 'closed' }
  | { kind: 'create' }
  | { kind: 'edit'; subscription: SubscriptionDto }

type SubscriptionsPanelProps = {
  subscriptionsState: FetchState<SubscriptionsResponse>
  billingChargesState: FetchState<BillingChargesResponse>
  onMutated: () => void
}

/// Group imported subscription-category charges into one row per
/// (amount, description). The dashboard displays each group like a
/// virtual subscription — e.g. "Claude Pro · $21.60 × 5 charges since
/// 2025-10-20" — instead of 5 separate line items. Same amount with
/// different descriptions stays separate, since that probably means
/// the user re-tagged some rows during import.
type ImportedSubscriptionGroup = {
  amountUsd: number
  description: string
  chargeCount: number
  earliestDate: string
  latestDate: string
  totalUsd: number
}

function groupImportedSubscriptions(
  charges: BillingChargeRow[],
): ImportedSubscriptionGroup[] {
  const buckets = new Map<string, ImportedSubscriptionGroup>()
  for (const charge of charges) {
    if (charge.category !== 'subscription') continue
    const description = charge.description ?? ''
    // Round amount to cents so $21.60 and $21.6 collapse to one
    // bucket; tax-quirky cents in the future won't fracture.
    const roundedAmount = Math.round(charge.amount_usd * 100) / 100
    const key = `${roundedAmount.toFixed(2)}|${description}`
    const existing = buckets.get(key)
    if (existing) {
      existing.chargeCount += 1
      existing.totalUsd += charge.amount_usd
      if (charge.occurred_at < existing.earliestDate)
        existing.earliestDate = charge.occurred_at
      if (charge.occurred_at > existing.latestDate)
        existing.latestDate = charge.occurred_at
    } else {
      buckets.set(key, {
        amountUsd: roundedAmount,
        description,
        chargeCount: 1,
        earliestDate: charge.occurred_at,
        latestDate: charge.occurred_at,
        totalUsd: charge.amount_usd,
      })
    }
  }
  return Array.from(buckets.values()).sort((a, b) => b.amountUsd - a.amountUsd)
}

function SubscriptionsPanel({
  subscriptionsState,
  billingChargesState,
  onMutated,
}: SubscriptionsPanelProps) {
  const [formMode, setFormMode] = useState<FormMode>({ kind: 'closed' })
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const subscriptions =
    subscriptionsState.status === 'ok' ? subscriptionsState.data.subscriptions : []

  async function submitForm(formData: SubscriptionFormData) {
    setSubmitting(true)
    setFormError(null)
    try {
      const path =
        formMode.kind === 'edit'
          ? `/api/v1/subscriptions/${formMode.subscription.id}`
          : '/api/v1/subscriptions'
      const method = formMode.kind === 'edit' ? 'PUT' : 'POST'
      const response = await fetch(path, {
        method,
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
      setFormMode({ kind: 'closed' })
      onMutated()
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
      onMutated()
    } catch (error) {
      window.alert(`Could not delete subscription: ${(error as Error).message}`)
    }
  }

  return (
    <section className="bg-white rounded-lg border border-slate-200 p-5 space-y-4">
      <div className="flex items-baseline justify-between">
        <h2 className="text-base font-medium">Subscriptions</h2>
        {formMode.kind === 'closed' && (
          <button
            type="button"
            className="text-sm text-blue-600 hover:underline"
            onClick={() => {
              setFormMode({ kind: 'create' })
              setFormError(null)
            }}
          >
            + Add subscription
          </button>
        )}
      </div>

      {formMode.kind !== 'closed' && (
        <SubscriptionForm
          // Force remount when switching create→edit→another-edit so the
          // form's internal state always starts fresh from initialValues.
          key={formMode.kind === 'edit' ? `edit-${formMode.subscription.id}` : 'create'}
          mode={formMode.kind}
          initialValues={formMode.kind === 'edit' ? formMode.subscription : null}
          submitting={submitting}
          error={formError}
          onCancel={() => {
            setFormMode({ kind: 'closed' })
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
      {subscriptionsState.status === 'ok' &&
        subscriptions.length === 0 &&
        formMode.kind === 'closed' &&
        // Suppress the empty hint when imported subscription charges
        // exist — the imported section below already shows the user
        // their subscriptions, just sourced from a different ingest
        // path. "No subscriptions yet" would be a lie in that case.
        (billingChargesState.status !== 'ok' ||
          !billingChargesState.data.charges.some(
            (charge) => charge.category === 'subscription',
          )) && (
          <div className="text-sm text-slate-500">
            No subscriptions yet. Add Claude Max, a Team seat, or any other flat-fee plan to see
            net value over your selected date range. (Or import a billing CSV below — recurring
            charges tagged as subscriptions show up automatically.)
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
              <div className="flex items-center gap-3 text-xs">
                <button
                  type="button"
                  className="text-blue-600 hover:underline"
                  onClick={() => {
                    setFormMode({ kind: 'edit', subscription: sub })
                    setFormError(null)
                  }}
                >
                  Edit
                </button>
                <button
                  type="button"
                  className="text-rose-600 hover:underline"
                  onClick={() => {
                    if (window.confirm(`Delete subscription "${sub.plan_name}"?`)) {
                      void deleteSubscription(sub.id)
                    }
                  }}
                >
                  Delete
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {billingChargesState.status === 'ok' &&
        (() => {
          const groups = groupImportedSubscriptions(billingChargesState.data.charges)
          if (groups.length === 0) return null
          // The pt-2 + border-t pair separates this section from the
          // manual subscriptions list above. Drop both when the manual
          // list is empty — otherwise it's a divider above nothing.
          const dividerClass =
            subscriptions.length > 0 ? 'pt-2 border-t border-slate-100' : ''
          return (
            <div className={`space-y-2 ${dividerClass}`.trim()}>
              <div className="text-xs font-medium text-slate-500 uppercase tracking-wide">
                Imported subscription charges
              </div>
              <ul className="divide-y divide-slate-100">
                {groups.map((group) => (
                  <li
                    key={`${group.amountUsd}|${group.description}`}
                    className="py-2 text-sm"
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-medium text-slate-900">
                        {group.description || '(no description)'}
                      </span>
                      <span className="text-slate-500 text-xs">
                        {group.chargeCount} charge{group.chargeCount === 1 ? '' : 's'} ·{' '}
                        {formatExactDollars(group.totalUsd)} total
                      </span>
                    </div>
                    <div className="text-xs text-slate-500">
                      {formatExactDollars(group.amountUsd)} per charge · since{' '}
                      {group.earliestDate}
                      {group.earliestDate === group.latestDate
                        ? ''
                        : ` (most recent ${group.latestDate})`}
                    </div>
                  </li>
                ))}
              </ul>
              <p className="text-[11px] text-slate-400">
                Sourced from imported billing data. Re-categorize individual charges from
                the import panel above if any of these were mis-tagged.
              </p>
            </div>
          )
        })()}
    </section>
  )
}

// ---------------------------------------------------------------------------
// SubscriptionForm — used in both create and edit modes.
// ---------------------------------------------------------------------------

type SubscriptionFormData = {
  planName: string
  monthlyUsd: number
  startedAt: string
  endedAt: string | null
}

type SubscriptionFormProps = {
  mode: 'create' | 'edit'
  initialValues: SubscriptionDto | null
  submitting: boolean
  error: string | null
  onCancel: () => void
  onSubmit: (data: SubscriptionFormData) => void
}

/**
 * Plan templates the user can pick from to pre-fill the form. Values are
 * Anthropic's published list rates as known to this build's training data;
 * the user can still edit any pre-filled value before saving. "Custom"
 * is the no-op default.
 *
 * If Anthropic adds or repricers a tier, update this list — there is no
 * config-file equivalent yet (though the `pricing.toml` discipline is
 * the obvious upgrade path if these go stale).
 */
const SUBSCRIPTION_PLAN_TEMPLATES: ReadonlyArray<{
  id: string
  label: string
  planName: string
  monthlyUsd: number | null
}> = [
  { id: 'custom', label: 'Custom (enter values manually)', planName: '', monthlyUsd: null },
  { id: 'pro', label: 'Claude Pro · $20/mo', planName: 'Claude Pro', monthlyUsd: 20 },
  { id: 'max-5x', label: 'Claude Max 5× · $100/mo', planName: 'Claude Max 5×', monthlyUsd: 100 },
  {
    id: 'max-20x',
    label: 'Claude Max 20× · $200/mo',
    planName: 'Claude Max 20×',
    monthlyUsd: 200,
  },
  {
    id: 'team',
    label: 'Claude Team · $25/seat/mo',
    planName: 'Claude Team',
    monthlyUsd: 25,
  },
  {
    id: 'enterprise',
    label: 'Claude Enterprise · custom pricing',
    planName: 'Claude Enterprise',
    monthlyUsd: 0,
  },
]

function SubscriptionForm({
  mode,
  initialValues,
  submitting,
  error,
  onCancel,
  onSubmit,
}: SubscriptionFormProps) {
  const isEdit = mode === 'edit'
  const [planName, setPlanName] = useState(initialValues?.plan_name ?? 'Claude Max 20×')
  const [monthlyUsd, setMonthlyUsd] = useState<string>(
    initialValues ? String(initialValues.monthly_usd) : '200',
  )
  const [startedAt, setStartedAt] = useState<string>(
    initialValues?.started_at ?? isoDateDaysAgo(0),
  )
  const [endedAt, setEndedAt] = useState<string>(initialValues?.ended_at ?? '')
  const [templateId, setTemplateId] = useState<string>('custom')

  function applyTemplate(nextTemplateId: string) {
    setTemplateId(nextTemplateId)
    const template = SUBSCRIPTION_PLAN_TEMPLATES.find((t) => t.id === nextTemplateId)
    if (!template || template.monthlyUsd === null) return
    setPlanName(template.planName)
    setMonthlyUsd(String(template.monthlyUsd))
  }

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
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-slate-700 uppercase tracking-wide">
          {isEdit ? 'Edit subscription' : 'Add subscription'}
        </span>
        {!isEdit && (
          <label className="text-xs text-slate-600 flex items-center gap-2">
            <span>Pre-fill from plan:</span>
            <select
              value={templateId}
              onChange={(event) => applyTemplate(event.target.value)}
              className="border border-slate-300 rounded-md px-2 py-1 text-xs bg-white"
            >
              {SUBSCRIPTION_PLAN_TEMPLATES.map((template) => (
                <option key={template.id} value={template.id}>
                  {template.label}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>

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

      {!isEdit && (
        <p className="text-xs text-slate-500">
          Plan-template prices reflect Anthropic's list rates as known when this build was
          compiled — verify against{' '}
          <a
            className="underline"
            href="https://www.anthropic.com/pricing"
            target="_blank"
            rel="noreferrer"
          >
            anthropic.com/pricing
          </a>{' '}
          and adjust before saving if your plan differs.
        </p>
      )}

      {error && <div className="text-xs text-rose-700">{error}</div>}

      <div className="flex items-center gap-2">
        <button
          type="submit"
          disabled={submitting}
          className="bg-blue-600 hover:bg-blue-700 text-white text-sm rounded-md px-3 py-1 disabled:bg-slate-300"
        >
          {submitting ? 'Saving…' : isEdit ? 'Save changes' : 'Save'}
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

// ---------------------------------------------------------------------------
// MethodologyPage — the "how every number gets computed" surface.
// Fetches bundled markdown from the server (which `include_str!`'d each
// doc at build time) and renders with react-markdown + GFM. Four tabs:
// methodology narrative, bibliography, research log, open questions.
// All four docs are also visible directly on GitHub — the tap on this
// page is convenience, not data ownership.
// ---------------------------------------------------------------------------

type DocSlug = 'methodology' | 'sources' | 'research-log' | 'request-for-research'

const METHODOLOGY_TABS: ReadonlyArray<{
  slug: DocSlug
  label: string
  description: string
}> = [
  {
    slug: 'methodology',
    label: 'Methodology',
    description: 'How every environmental number gets computed.',
  },
  {
    slug: 'sources',
    label: 'Bibliography',
    description: 'Every factor source with confidence tag, access date, and summary.',
  },
  {
    slug: 'research-log',
    label: 'Research log',
    description: 'Audit trail of past factor-model sweeps.',
  },
  {
    slug: 'request-for-research',
    label: 'Open questions',
    description: 'What the next quarterly sweep should address.',
  },
]

function MethodologyPage() {
  const [activeTab, setActiveTab] = useState<DocSlug>('methodology')
  const [docContent, setDocContent] = useState<FetchState<string>>({ status: 'idle' })

  // Re-fetch on tab change. Docs are small (the largest is ~25KB);
  // no need to cache across tab switches for v0.1.
  useEffect(() => {
    let cancelled = false
    setDocContent({ status: 'loading' })
    fetch(`/api/v1/docs/${activeTab}`)
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        return await response.text()
      })
      .then((markdown) => {
        if (!cancelled) setDocContent({ status: 'ok', data: markdown })
      })
      .catch((error) => {
        if (!cancelled) {
          setDocContent({ status: 'error', message: (error as Error).message })
        }
      })
    return () => {
      cancelled = true
    }
  }, [activeTab])

  return (
    <main className="mx-auto max-w-4xl px-6 py-8 space-y-6">
      <section className="bg-white rounded-lg border border-slate-200 p-5 space-y-5">
        <header className="space-y-2">
          <h2 className="text-xl font-semibold tracking-tight">
            How tokenscale's numbers are computed
          </h2>
          <p className="text-sm text-slate-600">
            Every environmental-impact figure on the dashboard traces back to a published
            source and a documented derivation. This page is the audit trail — methodology
            narrative, source bibliography, research log, and open questions for future
            refinement.
          </p>
        </header>

        <nav className="flex flex-wrap gap-1 border-b border-slate-200 -mx-5 px-5">
          {METHODOLOGY_TABS.map((tab) => {
            const active = tab.slug === activeTab
            return (
              <button
                key={tab.slug}
                type="button"
                onClick={() => setActiveTab(tab.slug)}
                className={
                  'px-3 py-2 text-sm transition-colors border-b-2 -mb-px ' +
                  (active
                    ? 'border-blue-600 text-blue-700 font-medium'
                    : 'border-transparent text-slate-600 hover:text-slate-900 hover:border-slate-300')
                }
                title={tab.description}
              >
                {tab.label}
              </button>
            )
          })}
        </nav>

        {docContent.status === 'loading' && (
          <div className="text-sm text-slate-500 py-12 text-center">Loading…</div>
        )}
        {docContent.status === 'error' && (
          <div className="text-sm text-rose-700 py-4">
            Could not load this doc: {docContent.message}. The doc is also viewable on{' '}
            <a
              className="underline"
              href={`https://github.com/RobarePruyn/tokenscale/blob/main/docs/${activeTab}.md`}
              target="_blank"
              rel="noreferrer"
            >
              GitHub
            </a>
            .
          </div>
        )}
        {docContent.status === 'ok' && (
          <article className="prose-tokenscale">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{docContent.data}</ReactMarkdown>
          </article>
        )}
      </section>
    </main>
  )
}
