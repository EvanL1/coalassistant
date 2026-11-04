// 公开数据标定: 公开出厂含税报价 ← 当天钢铁网 7 条分煤种指数 (标准品 = 方法论附录B).
// 用法: node fit.mjs <public.json> <mcci.json> <master.json> <blend>
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const [pubPath, mcciPath, masterPath, bin] = process.argv.slice(2);
const pub = JSON.parse(readFileSync(pubPath, "utf8"));
const mcci = JSON.parse(readFileSync(mcciPath, "utf8")).series;
const master = JSON.parse(readFileSync(masterPath, "utf8"));
const pf = master.price_factor;
const num = (v) => (v === "" || v == null ? undefined : Number(v));

// 样本: 报价 (不含成交)、现金或未注明 (不含承兑)、指标是单值 (不是规格界限)、有 V.
export const samples = pub
  .filter((r) => r.price_type === "quote" && r.payment !== "acceptance" && r.province === "山西")
  .filter((r) => ["A_op", "S_op", "V_op", "G_op"].every((k) => r[k] === "eq") && num(r.V) != null)
  .map((r) => ({
    id: r.id, group: r.group_id, set: r.source_set, date: r.quote_date || r.quote_period_end,
    label: `${r.region}${r.subregion ? "/" + r.subregion : ""} ${r.coal_type}`,
    price: num(r.normalized_price_cny_t) ?? num(r.price),
    props: Object.fromEntries(["A", "S", "V", "G", "CSR"].map((k) => [k, r[`${k}_op`] === "eq" ? num(r[k]) : undefined]).filter(([, v]) => v != null)),
  }))
  .filter((s) => s.price > 0);

const indexOn = (date) => pf.index_series.map((s) => {
  const p = mcci.find((m) => m.code === s.code).points.filter((x) => x.date <= date).at(-1);
  return { name: s.name, coal_type: s.coal_type, props: s.props, price: p.price, date: p.date };
});
const byDate = Object.groupBy(samples, (s) => s.date);

export function evaluate(cfg) {
  const adjusts = pf.adjusts.map((a) => {
    const o = cfg.adj?.[a.indicator];
    const r = o ? { ...a, ...o } : a;
    return a.indicator === "CSR" ? { ...r, enabled: !!cfg.csr } : r;
  });
  const factor = { adjusts, matching: { weights: { V: cfg.v }, missing_penalty: 300, bandwidth: cfg.tau } };
  const out = [];
  for (const [date, list] of Object.entries(byDate)) {
    const req = { coals: list.map((s) => ({ name: s.id, props: s.props, fob: 0, frt: 0 })), price_factor: { ...factor, benchmarks: indexOn(date) } };
    const res = JSON.parse(execFileSync(bin, ["price"], { input: JSON.stringify(req) }).toString());
    if (!res.ok) throw new Error(res.reason);
    for (const v of res.coal_values) { const s = list.find((x) => x.id === v.name); out.push({ ...s, est: v.fair, err: v.fair - s.price, mix: v.benchmark_mix }); }
  }
  return out;
}
// 按产品组等权 (周报同一组连出三期, 不让它占三票).
export function score(rows, filter = () => true) {
  const g = Object.groupBy(rows.filter(filter), (r) => r.group);
  const per = Object.values(g).map((rs) => ({ abs: rs.reduce((a, r) => a + Math.abs(r.err), 0) / rs.length, bias: rs.reduce((a, r) => a + r.err, 0) / rs.length }));
  return { mae: per.reduce((a, x) => a + x.abs, 0) / per.length, bias: per.reduce((a, x) => a + x.bias, 0) / per.length, groups: per.length };
}
