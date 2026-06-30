// 把 coal_master.json (112 座) 迁移进 mines 宽表 schema.
// 输出 INSERT SQL 到 stdout,管道给 sqlite3 执行。
// region 机械拆成 province/city;props→各列;confidence→可信度表;lat/lng 留空。
import { readFileSync } from "node:fs";

const SRC = "/Users/lyf/dev/coalassistant/blend_kit_rs/data/coal_master.json";
const master = JSON.parse(readFileSync(SRC, "utf8"));

// 按长度降序匹配省名前缀(先匹配"内蒙古"再匹配"山西")
const PROVINCES = ["内蒙古", "黑龙江", "山西", "陕西", "河北", "河南", "山东",
  "宁夏", "新疆", "甘肃", "青海", "贵州", "云南", "四川", "安徽", "辽宁", "吉林", "重庆"];
const PROP = { S: "s", A: "a", V: "v", G: "g", Y: "y", petro: "petro", CSR: "csr", M: "m" };
const CONF = { ...PROP, fob: "fob", frt: "frt" };

const q = (s) => (s == null ? "NULL" : `'${String(s).replace(/'/g, "''")}'`);
const num = (x) => (x == null || x === "" ? "NULL" : Number(x));

function splitRegion(r) {
  if (!r) return [null, null];
  const p = PROVINCES.find((p) => r.startsWith(p));
  if (!p) return [null, r];
  return [p, r.slice(p.length).trim() || null];
}

const out = ["BEGIN;"];
for (const c of master.coals) {
  const [province, city] = splitRegion(c.region);
  const p = c.props || {};
  const cols = "name,coal_type,status,province,city,s,a,v,g,y,petro,csr,m,fob,frt,note";
  const vals = [
    q(c.name), q(c.coal_type), q(c.status || "incomplete"),
    q(province), q(city),
    num(p.S), num(p.A), num(p.V), num(p.G), num(p.Y), num(p.petro), num(p.CSR), num(p.M),
    num(c.fob), num(c.frt), q(c.note),
  ];
  out.push(`INSERT INTO mines (${cols}) VALUES (${vals.join(",")});`);

  for (const [k, conf] of Object.entries(c.confidence || {})) {
    const f = CONF[k];
    if (!f || conf == null) continue;
    out.push(
      `INSERT INTO mine_field_confidence (mine_id, field, confidence) ` +
      `VALUES ((SELECT id FROM mines WHERE name=${q(c.name)}), '${f}', ${q(conf)});`
    );
  }
}
out.push("COMMIT;");
process.stdout.write(out.join("\n") + "\n");
