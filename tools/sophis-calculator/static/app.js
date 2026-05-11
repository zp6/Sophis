// Sophis Energy Offset Calculator — pure client-side computation.
// See docs/H1_CALCULATOR_DESIGN.md §3.2 for the canonical formulas.

(function () {
  "use strict";

  const inputs = ["hashrate", "power", "hours", "intensity", "creditprice", "coinbase", "sphsprice"];
  const out = {
    energy: document.getElementById("energy-out"),
    co2: document.getElementById("co2-out"),
    cost: document.getElementById("cost-out"),
    pct: document.getElementById("pct-out"),
    pctCmd: document.getElementById("pct-cmd"),
    suggestion: document.getElementById("suggestion-card"),
  };

  const DAYS_PER_MONTH = 30;

  function num(id) {
    const v = parseFloat(document.getElementById(id).value);
    return Number.isFinite(v) ? Math.max(0, v) : 0;
  }

  function fmt(n, digits) {
    if (!Number.isFinite(n)) return "0";
    return n.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });
  }

  function recompute() {
    const power_w = num("power");
    const hours = num("hours");
    const intensity = num("intensity");
    const creditPrice = num("creditprice");
    const coinbase = num("coinbase");
    const sphsPrice = num("sphsprice");

    // Core formulas (D2 of DESIGN §3.2):
    const kwh = (power_w * hours * DAYS_PER_MONTH) / 1000;
    const co2_kg = (kwh * intensity) / 1000;
    const cost_usd = (co2_kg / 1000) * creditPrice;

    out.energy.textContent = fmt(kwh, 2);
    out.co2.textContent = fmt(co2_kg, 2);
    out.cost.textContent = fmt(cost_usd, 2);

    // Suggested-percent line: only if both coinbase + price are positive.
    const monthlyUsd = coinbase * sphsPrice;
    if (monthlyUsd > 0 && cost_usd > 0) {
      const pct = Math.min(100, Math.ceil((cost_usd / monthlyUsd) * 100));
      out.pct.textContent = pct;
      out.pctCmd.textContent = pct;
      out.suggestion.hidden = false;
    } else {
      out.suggestion.hidden = true;
    }
  }

  // Wire all inputs to recompute on every change. Use 'input' (not
  // 'change') for live feedback as the user types.
  inputs.forEach(function (id) {
    const el = document.getElementById(id);
    if (el) el.addEventListener("input", recompute);
  });

  // Initial render.
  recompute();
})();
