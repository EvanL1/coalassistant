import { writeFileSync } from "node:fs";
import { samples, evaluate, score } from "./fit.mjs";
const res = [];
for (const gp of [0, 2.5, 5]) for (const gd of [5, 8, 10, 12]) for (const a of [25, 36]) for (const sd of [30, 47]) for (const v of [10, 20]) for (const tau of [40, 80]) {
  const cfg = { v, tau, adj: { G: { premium_rate: gp, discount_rate: gd }, A: { premium_rate: a, discount_rate: a }, S: { discount_rate: sd } } };
  const rows = evaluate(cfg);
  res.push({ key: `G+${gp}/-${gd} A${a} Sd${sd} V${v} τ${tau}`, all: score(rows), ms: score(rows, (r) => r.set === "mysteel"), rp: score(rows, (r) => r.set === "reports") });
}
const f = (s) => `${s.mae.toFixed(0)}/${s.bias >= 0 ? "+" : ""}${s.bias.toFixed(0)}`;
// 两组来源都要好: 取两边 MAE 的较大者最小.
res.forEach((r) => (r.worst = Math.max(r.ms.mae, r.rp.mae)));
res.sort((a, b) => a.worst - b.worst);
console.log("配置\t\t\t\t全部\t快讯\t周报");
for (const r of res.slice(0, 15)) console.log(`${r.key.padEnd(32)}\t${f(r.all)}\t${f(r.ms)}\t${f(r.rp)}`);
writeFileSync("grid2.json", JSON.stringify(res));
