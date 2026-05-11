# H1 — Energy Offset Calculator

> **Status:** design frozen for sub-fase H1.0 — ready for H1.1 implementation.
> **Originating roadmap:** Roadmap H (Energy compensation pillar), item H1.
> **Companion docs:** future `docs/H1_RUNBOOK.md` (deferred follow-up).
> **Pre-existing baseline:** **none**. Sophis has no on-domain calculator
> today. The `--donate-percent` flag exists (`sophis-miner` commit
> `e54fcd9`, 2026-05-05) but operators have no canonical way to compute
> "how much should I donate?" — H1 fills that gap.

## 1. Motivation

The energy-compensation narrative pillar (decided 2026-05-07; see
`project_energy_compensation_narrative.md`) defines six components.
H1 is component #1: a public web calculator at `sophis.org/calculator`
that takes miner inputs and returns the energy-impact numbers
operators need to choose a `--donate-percent`.

Without it:

- **Operators have no anchor for "how much should I donate".** They
  guess, default to 0%, or copy whatever they see on a forum.
- **The narrative becomes vapor.** Saying "miners can compensate via
  `--donate-percent`" has no force without a canonical way to compute
  the percent.
- **The discussion has no honest baseline.** Any debate about "is PoW
  wasteful?" needs concrete numbers per-miner, not abstractions.

The calculator quantifies. It does **not** recommend destinations
(per Decisão 6 of the 2026-05-04 pivot — equipe non-custodial, sem
curadoria) and **does not** call any external APIs (no privacy
exposure of miner stats). All computation is client-side.

This is item #7 in the sequential roadmap (`project_roadmap_sequence_2026_05_09.md`)
and the lightest deliverable on the list (~1-2 days).

## 2. Ratified design decisions

| ID | Question | Choice | Rationale |
|----|----------|--------|-----------|
| **D1** | Computation location | Client-side JS only (no backend, no API calls) | Privacy: miner inputs (hashrate, TDP, hours) leak nothing if computed in-browser. Operational: no backend = no operating cost = no operator-key dependency. Trade-off: cannot use real-time grid carbon intensity APIs (electricitymaps.com, WattTime); we ship a single global-average default. Operators who want jurisdiction-specific numbers can override the carbon-intensity input themselves. |
| **D2** | Default carbon intensity | 480 g CO2-eq / kWh (global grid average) | IEA 2023 global average is ~480 g/kWh. Using a single conservative default avoids the regulatory rabbit hole of "which jurisdiction?" — operators can override. The number is a UI default, not a normative claim about Sophis miners. |
| **D3** | Default carbon credit price | $50 USD / metric ton CO2-eq | Mid-2024 voluntary carbon market price hovered $40-60/ton for verified projects (Verra, Gold Standard). $50 is a round midpoint. Operators override per their preferred provider. The number frames *order of magnitude*, not a quote. |
| **D4** | Destination recommendations | **None.** Calculator displays USD cost only — never an address, NGO name, or "where to donate". | Decisão 6 (Operational Boundaries): Sophis core team does not curate. Calculator shows "your impact is X kg CO2 / Y USD"; *what* you do with that info is your choice. Listing destinations would re-create the curation surface H2 was rejected to avoid (`project_h2_rejected.md`). |
| **D5** | Suggested-percent formula | `donate_percent = ceil(min(100, carbon_credit_cost_usd / monthly_coinbase_usd_estimate * 100))` | Calculator UI displays a "to compensate 100% of your impact, redirect ~X% of your coinbase" line. Caller must supply (a) expected SPHS/month and (b) SPHS price for the estimate. Both are user-facing inputs with sensible defaults; the calculator is transparent that this is *one* possible target (compensate 100%) — operators may choose less. |
| **D6** | Hosting | Static file deploy + tiny optional axum server for local dev | Static HTML+JS deploys to GitHub Pages or any CDN with zero ops. The axum server in `tools/sophis-calculator/` is a developer convenience (single `cargo run` to view locally) and embeds the static files via `include_str!` — same pattern as `tools/sophis-dashboard/`. |

## 3. Computation

### 3.1 Inputs

| Field | Unit | Default | Notes |
|-------|------|---------|-------|
| Hashrate | kH/s | 1.0 | RandomX hashrate; informational (does not enter the energy formula directly). |
| Power draw | W | 65 | TDP of a typical desktop CPU under sustained RandomX load. |
| Hours/day | h | 24 | Default to always-on; downscale for laptop / part-time miners. |
| Carbon intensity | g CO2-eq / kWh | 480 | Global grid average (D2). Override per jurisdiction. |
| Carbon credit price | USD / ton CO2-eq | 50 | Voluntary market midpoint (D3). Override per provider. |
| Expected coinbase | SPHS / month | 0 | Optional — leave blank to skip the suggested-percent line. |
| SPHS price | USD / SPHS | 0 | Optional — required only if expected coinbase is set. |

### 3.2 Formulas

```text
energy_kwh_per_month        = power_w * hours_per_day * 30 / 1000
co2_kg_per_month            = energy_kwh_per_month * carbon_intensity_g_per_kwh / 1000
carbon_credit_cost_usd      = co2_kg_per_month / 1000 * carbon_credit_price_per_ton
                            // (kg → ton: ÷1000)

monthly_coinbase_usd_est    = expected_coinbase_sphs * sphs_price_usd
                            // skipped if either input is 0

suggested_donate_percent    = if monthly_coinbase_usd_est > 0:
                                ceil(min(100, carbon_credit_cost_usd /
                                              monthly_coinbase_usd_est * 100))
                              else: undefined (UI hides line)
```

### 3.3 Worked example

A miner running 65 W for 24 h/day at the global default carbon
intensity, with expected coinbase 100 SPHS/mo at $0.50/SPHS:

```text
energy_kwh = 65 * 24 * 30 / 1000     = 46.8 kWh/mo
co2_kg     = 46.8 * 480 / 1000       = 22.46 kg CO2/mo
credit_usd = 22.46 / 1000 * 50       = $1.12 / mo

coinbase_usd_est = 100 * 0.50        = $50/mo
suggested_pct    = ceil(1.12 / 50 * 100) = 3%
```

UI shows: "Your monthly impact is **22.5 kg CO2** ≈ **$1.12** in
voluntary carbon credits. To compensate 100% of this impact, redirect
**~3%** of your coinbase via `--donate-percent 3`."

## 4. Frozen ABI surface (such as it is)

There is no on-chain consensus impact, no RPC, no SDK API. The
"frozen surface" is purely the URL + the documented formulas.

| Item | Value |
|------|-------|
| Canonical hosting URL | `sophis.org/calculator` |
| Default carbon intensity | `480 g/kWh` |
| Default carbon credit price | `$50/ton` |
| Default power draw | `65 W` |
| Default hours/day | `24 h` |
| Default month length | `30 days` |
| Suggested-percent ceiling | `100%` (cap at full coinbase redirection) |

Any of these defaults can change without breaking anyone — they are UI
hints, not commitments. A future revision (call it H1.5) might swap
the global average for a real-time API call to `electricitymaps.com`,
gated behind a "high-precision mode" toggle that requires explicit
user opt-in to the privacy trade-off.

## 5. Out-of-scope (for H1)

- **Real-time grid intensity APIs** (electricitymaps.com, WattTime) —
  privacy trade-off + operator dependency. Defer to a future SIP
  with explicit user opt-in.
- **Per-miner historical tracking** — no backend = no storage.
  Operators who want this can self-host with the static files +
  their own backend.
- **NGO / project recommendations** — Decisão 6 (D4 above).
- **Direct integration with `--donate-to`** (e.g. "click here to
  generate the miner command line") — would imply destination
  recommendation. Out.
- **i18n / translations** — single-language English MVP. PT-BR
  follow-up trivial.

## 6. Reference implementation map

| Sub-fase | Scope |
|---------|-------|
| H1.0 | This design document |
| H1.1 | `tools/sophis-calculator/static/{index.html, app.js, style.css}` — pure client-side calculator |
| H1.2 | `tools/sophis-calculator/` — tiny axum binary serving the embedded static files for local dev |
| H1.3 | Workspace check + clippy strict + single commit |

## 7. Glossary

| Term | Meaning |
|------|---------|
| Carbon intensity | Grams of CO2-equivalent emitted per kilowatt-hour of grid electricity, jurisdiction-dependent. The calculator default `480 g/kWh` is the IEA 2023 global average. |
| Voluntary carbon credit | A tradable certificate representing 1 metric ton of CO2-eq removed or avoided. Voluntary market prices range $5-100+ per ton depending on project type (afforestation, direct air capture, methane capture). The calculator default `$50/ton` is a round midpoint. |
| `--donate-percent` | The opt-in client-side flag on `sophis-miner` (commit `e54fcd9`, 2026-05-05) that splits the coinbase between the miner address and one or more donation addresses chosen by the operator. Defaults to 0% (full coinbase to miner). |
| Compensation target | The percentage of impact a miner chooses to offset via `--donate-percent`. The calculator suggests the value that fully covers carbon credit cost; operators may choose less or more. |
