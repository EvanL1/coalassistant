/**
 * 管理端 - 基准报价.
 *
 * 估价模块按"同煤种、指标最接近"的基准报价换算每种煤的价, 所以基准越新、煤种越全,
 * 估价越准. 这里列出当前生效的基准 (master 默认值 + 补录, 同名取最新报价日), 并补录
 * 新报价: 拿到某个品种的最新价 (钢铁网快讯、矿上报价都行) 就录一条, 不用重新部署.
 * 报价日早于焦煤现货指数最新一期的, 服务端按指数涨跌推算到今天, 原价照旧显示.
 */
import { useEffect, useState } from "react";
import {
  createBenchmark,
  deleteBenchmark,
  fetchBenchmarks,
} from "../benchmarks";
import { coalTag } from "../domain/coalTypes";
import type { Benchmark, BenchmarkEntry } from "../types";

const SPEC_KEYS = ["S", "A", "V", "G", "Y"] as const;
const COAL_TYPES = ["焦煤", "肥煤", "1/3焦煤", "瘦煤", "贫瘦煤", "气煤"];
const FORMS = ["精煤", "筛选精煤", "浮选精煤", "中煤", "煤泥", "原煤"];

const ORIGIN_LABEL: Record<NonNullable<BenchmarkEntry["origin"]>, string> = {
  master: "默认",
  stored: "补录",
  mcci: "钢铁网指数",
};

const small = { fontSize: 11, color: "var(--c-text-3)" };
const cell = { padding: "4px 6px 4px 0", verticalAlign: "top" as const };

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

function specText(props: BenchmarkEntry["props"]): string {
  return SPEC_KEYS.filter((key) => props[key] != null)
    .map((key) => `${key}${props[key]}`)
    .join(" ");
}

interface Draft {
  name: string;
  coal_type: string;
  form: string;
  region: string;
  price: string;
  date: string;
  source: string;
  props: Record<(typeof SPEC_KEYS)[number], string>;
}

function emptyDraft(): Draft {
  return {
    name: "",
    coal_type: "焦煤",
    form: "精煤",
    region: "",
    price: "",
    date: today(),
    source: "",
    props: { S: "", A: "", V: "", G: "", Y: "" },
  };
}

/** 草稿 → 请求体; 填错返回原因. 空指标不带 (估价时跳过该项). */
export function draftToBenchmark(draft: Draft): Benchmark | string {
  const name = draft.name.trim();
  if (!name) return "请填品名";
  const price = Number(draft.price);
  if (!Number.isFinite(price) || price <= 0) return "报价须为正数";
  const props: Benchmark["props"] = {};
  for (const key of SPEC_KEYS) {
    const raw = draft.props[key].trim();
    if (!raw) continue;
    const value = Number(raw);
    if (!Number.isFinite(value)) return `${key} 不是数字`;
    props[key] = value;
  }
  if (props.S == null) return "至少要填硫分 S";
  return {
    name,
    coal_type: draft.coal_type,
    form: draft.form,
    region: draft.region.trim() || undefined,
    props,
    price,
    date: draft.date,
    source: draft.source.trim() || undefined,
  };
}

export function BenchmarkCard() {
  const [rows, setRows] = useState<BenchmarkEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [saving, setSaving] = useState(false);

  async function refresh() {
    setRows(await fetchBenchmarks());
  }

  useEffect(() => {
    let alive = true;
    fetchBenchmarks().then((list) => alive && setRows(list));
    return () => {
      alive = false;
    };
  }, []);

  async function handleCreate() {
    const benchmark = draftToBenchmark(draft);
    if (typeof benchmark === "string") {
      setError(benchmark);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await createBenchmark(benchmark);
      setDraft(emptyDraft());
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(row: BenchmarkEntry) {
    if (!row.id || !confirm(`删掉补录的「${row.name} ${row.date}」?`)) return;
    setError(null);
    try {
      await deleteBenchmark(row.id);
      await refresh();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const input = (
    value: string,
    onChange: (value: string) => void,
    placeholder: string,
    width = 56,
  ) => (
    <input
      className="input"
      style={{ width, height: 30, fontSize: 12, padding: "0 6px" }}
      value={value}
      placeholder={placeholder}
      aria-label={`基准${placeholder}`}
      onChange={(e) => onChange(e.target.value)}
    />
  );

  return (
    <div className="card">
      <div className="card-title">基准报价</div>
      <p style={{ ...small, margin: "0 0 12px" }}>
        估价按 煤种 → 形态 → 地区 逐层找同类基准, 再取指标最接近的一条, 按质量指数之比换算.
        基准越新、品类越全, 估价越准. 地区写到市 (如"山西临汾"), 与煤库写法一致才对得上.
        拿到某个品种的最新出厂含税价就补录一条 (同名取最新报价日); 报价日早于焦煤现货指数最新一期的,
        按指数涨跌推算到今天.
      </p>

      {rows == null ? (
        <p style={small}>加载中…</p>
      ) : rows.length === 0 ? (
        <p style={small}>还没有基准报价, 估价退回现货指数点位 (只是相对尺度).</p>
      ) : (
        <table style={{ width: "100%", fontSize: 12, borderCollapse: "collapse" }}>
          <thead>
            <tr style={small}>
              <th style={{ ...cell, textAlign: "left", fontWeight: "normal" }}>煤种 · 品名</th>
              <th style={{ ...cell, textAlign: "left", fontWeight: "normal" }}>指标</th>
              <th style={{ ...cell, textAlign: "right", fontWeight: "normal" }}>报价</th>
              <th style={{ ...cell, textAlign: "left", fontWeight: "normal" }}>日期</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.name}>
                <td style={cell}>
                  <span style={small}>
                    {[coalTag(row) || "未分煤种", row.region].filter(Boolean).join(" · ")}
                  </span>{" "}
                  {row.name}
                  {row.source && <div style={small}>{row.source}</div>}
                </td>
                <td style={cell}>{specText(row.props)}</td>
                <td style={{ ...cell, textAlign: "right" }}>
                  {row.price.toFixed(0)}
                  {row.current_price != null && (
                    <div style={small}>
                      今 ≈ {row.current_price.toFixed(0)}
                    </div>
                  )}
                  {row.current_price != null && row.index_source && (
                    <div style={small}>按 {row.index_source}</div>
                  )}
                </td>
                <td style={cell}>
                  {row.date}
                  <div style={small}>{ORIGIN_LABEL[row.origin ?? "master"]}</div>
                </td>
                <td style={cell}>
                  {row.origin === "stored" && row.id && (
                    <button
                      className="btn btn-secondary"
                      style={{ height: 26, padding: "0 8px", fontSize: 11 }}
                      onClick={() => void handleDelete(row)}
                    >
                      删除
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <div style={{ marginTop: 12, display: "flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}>
        <select
          className="input"
          style={{ height: 30, fontSize: 12 }}
          aria-label="基准煤种"
          value={draft.coal_type}
          onChange={(e) => setDraft({ ...draft, coal_type: e.target.value })}
        >
          {COAL_TYPES.map((type) => (
            <option key={type} value={type}>
              {type}
            </option>
          ))}
        </select>
        <select
          className="input"
          style={{ height: 30, fontSize: 12 }}
          aria-label="基准形态"
          value={draft.form}
          onChange={(e) => setDraft({ ...draft, form: e.target.value })}
        >
          {FORMS.map((form) => (
            <option key={form} value={form}>
              {form}
            </option>
          ))}
        </select>
        {input(draft.name, (name) => setDraft({ ...draft, name }), "品名", 120)}
        {input(draft.region, (region) => setDraft({ ...draft, region }), "地区", 80)}
        {SPEC_KEYS.map((key) =>
          input(
            draft.props[key],
            (value) => setDraft({ ...draft, props: { ...draft.props, [key]: value } }),
            key,
            44,
          ),
        )}
        {input(draft.price, (price) => setDraft({ ...draft, price }), "出厂含税价", 80)}
        <input
          className="input"
          type="date"
          aria-label="基准报价日"
          style={{ height: 30, fontSize: 12 }}
          value={draft.date}
          onChange={(e) => setDraft({ ...draft, date: e.target.value })}
        />
        {input(draft.source, (source) => setDraft({ ...draft, source }), "来源", 120)}
        <button
          className="btn btn-primary"
          style={{ height: 30, padding: "0 12px", fontSize: 12 }}
          disabled={saving}
          onClick={() => void handleCreate()}
        >
          {saving ? "保存中…" : "补录"}
        </button>
      </div>
      {error && <p style={{ fontSize: 12, color: "var(--c-danger, #b91c1c)" }}>{error}</p>}
    </div>
  );
}
