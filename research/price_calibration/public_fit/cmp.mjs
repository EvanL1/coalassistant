import { evaluate } from "./fit.mjs";
const C = {
  现行: { v: 20, tau: 80 },
  G8: { v: 20, tau: 40, adj: { G: { premium_rate: 8, discount_rate: 8 }, A: { premium_rate: 25, discount_rate: 25 }, S: { discount_rate: 30 } } },
  G8lo10: { v: 20, tau: 40, adj: { G: { premium_rate: 8, discount_rate: 8, clamp: [10, 95] }, A: { premium_rate: 25, discount_rate: 25 }, S: { discount_rate: 30 } } },
};
const out = Object.fromEntries(Object.entries(C).map(([k, c]) => [k, evaluate(c)]));
const groups = Object.groupBy(out.现行, (r) => r.group);
console.log("产品组\t\t来源\t指标\t\t实价均\t" + Object.keys(C).map((k) => k + "差").join("\t"));
for (const [g, rs] of Object.entries(groups)) {
  const r = rs[0], avg = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length;
  const errs = Object.keys(C).map((k) => avg(out[k].filter((x) => x.group === g).map((x) => x.err)).toFixed(0));
  console.log(`${r.label.padEnd(14)}\t${r.set}\tV${r.props.V} G${r.props.G} S${r.props.S} A${r.props.A}${r.props.CSR ? " CSR" + r.props.CSR : ""}\t${avg(rs.map((x) => x.price)).toFixed(0)}(${rs.length})\t${errs.join("\t")}`);
}
