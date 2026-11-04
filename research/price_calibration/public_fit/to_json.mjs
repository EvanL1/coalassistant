// samples_comparable.csv → public.json (字段原样, 都是字符串). 用法: node to_json.mjs ../samples_comparable.csv > public.json
import { readFileSync } from "node:fs";
const text = readFileSync(process.argv[2], "utf8").replace(/^﻿/, "");
const rows = [];
let row = [], field = "", quoted = false;
for (let i = 0; i < text.length; i++) {
  const c = text[i];
  if (quoted) {
    if (c !== '"') field += c;
    else if (text[i + 1] === '"') { field += '"'; i++; }
    else quoted = false;
  } else if (c === '"') quoted = true;
  else if (c === ",") { row.push(field); field = ""; }
  else if (c === "\n") { row.push(field); rows.push(row); row = []; field = ""; }
  else if (c !== "\r") field += c;
}
if (field || row.length) { row.push(field); rows.push(row); }
const [header, ...body] = rows;
console.log(JSON.stringify(body.filter((r) => r.length === header.length).map((r) => Object.fromEntries(header.map((k, i) => [k, r[i]])))));
