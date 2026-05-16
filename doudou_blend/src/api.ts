/**
 * 跟 Cloudflare Pages Functions 的 API client.
 *
 * 设计:
 *   - 所有调用自动带 Authorization: Bearer <token> (token 从 localStorage)
 *   - 失败统一 throw, 让 storage 层决定要不要 fallback localStorage cache
 *   - 跑在 Tauri 桌面端时也能工作 (相对 URL → 走当前 origin), 但桌面端没后端,
 *     调用会失败而 storage 层会回退到 localStorage. 桌面端要真正多设备同步需要
 *     额外配置 API base URL, 留下一波.
 */

import type {
  Contract,
  Customer,
  MasterCoalEntry,
  Payment,
  Quote,
  Shipment,
} from "./types";

const KEY_API_TOKEN = "doudou_blend.api_token.v1";

export function getApiToken(): string | null {
  return localStorage.getItem(KEY_API_TOKEN);
}

export function setApiToken(token: string): void {
  localStorage.setItem(KEY_API_TOKEN, token);
}

export function clearApiToken(): void {
  localStorage.removeItem(KEY_API_TOKEN);
}

async function authFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const token = getApiToken();
  if (!token) throw new Error("未登录");
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${token}`);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  return fetch(path, { ...init, headers });
}

export interface LoginResp {
  ok: boolean;
  token?: string;
  error?: string;
}

/** 远程登录: 返回 token, 调用方负责存到 localStorage (用 setApiToken). */
export async function apiLogin(user: string, pass: string): Promise<LoginResp> {
  const resp = await fetch("/api/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ user, pass }),
  });
  return (await resp.json()) as LoginResp;
}

export async function apiListCoals(): Promise<MasterCoalEntry[]> {
  const resp = await authFetch("/api/coals");
  if (!resp.ok) throw new Error(`GET /api/coals 失败: ${resp.status}`);
  const data = (await resp.json()) as { coals: MasterCoalEntry[] };
  return data.coals;
}

export async function apiAddCoal(coal: MasterCoalEntry): Promise<void> {
  const resp = await authFetch("/api/coals", {
    method: "POST",
    body: JSON.stringify(coal),
  });
  if (!resp.ok) {
    const data = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(data.error || `POST /api/coals 失败: ${resp.status}`);
  }
}

export async function apiDeleteCoal(name: string): Promise<void> {
  const resp = await authFetch(
    `/api/coals?name=${encodeURIComponent(name)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/coals 失败: ${resp.status}`);
}

export async function apiMigrateCoals(coals: MasterCoalEntry[]): Promise<number> {
  const resp = await authFetch("/api/coals/migrate", {
    method: "POST",
    body: JSON.stringify({ coals }),
  });
  if (!resp.ok) throw new Error(`POST /api/coals/migrate 失败: ${resp.status}`);
  const data = (await resp.json()) as { imported: number };
  return data.imported;
}

// ============================================================
// 通用 KV settings (coal_prefs / user_contract / history)
// ============================================================

export async function apiGetSetting(key: string): Promise<string | null> {
  const resp = await authFetch(
    `/api/settings?key=${encodeURIComponent(key)}`,
  );
  if (!resp.ok) throw new Error(`GET /api/settings 失败: ${resp.status}`);
  const data = (await resp.json()) as { value: string | null };
  return data.value;
}

export async function apiPutSetting(key: string, value: string): Promise<void> {
  const resp = await authFetch("/api/settings", {
    method: "PUT",
    body: JSON.stringify({ key, value }),
  });
  if (!resp.ok) throw new Error(`PUT /api/settings 失败: ${resp.status}`);
}

export async function apiDeleteSetting(key: string): Promise<void> {
  const resp = await authFetch(
    `/api/settings?key=${encodeURIComponent(key)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/settings 失败: ${resp.status}`);
}

// ============================================================
// Customers
// ============================================================

export async function apiListCustomers(): Promise<Customer[]> {
  const resp = await authFetch("/api/customers");
  if (!resp.ok) throw new Error(`GET /api/customers 失败: ${resp.status}`);
  const data = (await resp.json()) as { customers: Customer[] };
  return data.customers;
}

export async function apiUpsertCustomer(c: Customer): Promise<void> {
  const resp = await authFetch("/api/customers", {
    method: "POST",
    body: JSON.stringify(c),
  });
  if (!resp.ok) {
    const e = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(e.error || `POST /api/customers 失败: ${resp.status}`);
  }
}

export async function apiDeleteCustomer(id: string): Promise<void> {
  const resp = await authFetch(
    `/api/customers?id=${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/customers 失败: ${resp.status}`);
}

// ============================================================
// Quotes
// ============================================================

interface QuoteWire {
  id: string;
  customer_id: string;
  customer_name: string;
  recipe_json: string;
  cost_cif: number;
  markup: number;
  quoted_price: number;
  total_tons: number | null;
  contract_name: string | null;
  status: string;
  note: string | null;
  created_at?: string;
  updated_at?: string;
}

function wireToQuote(w: QuoteWire): Quote {
  let recipe: Record<string, number> = {};
  try {
    recipe = JSON.parse(w.recipe_json) as Record<string, number>;
  } catch {
    recipe = {};
  }
  return {
    id: w.id,
    customer_id: w.customer_id,
    customer_name: w.customer_name,
    recipe,
    cost_cif: w.cost_cif,
    markup: w.markup,
    quoted_price: w.quoted_price,
    total_tons: w.total_tons,
    contract_name: w.contract_name,
    status: w.status as Quote["status"],
    note: w.note,
    created_at: w.created_at,
    updated_at: w.updated_at,
  };
}

function quoteToWire(q: Quote): QuoteWire {
  return {
    id: q.id,
    customer_id: q.customer_id,
    customer_name: q.customer_name,
    recipe_json: JSON.stringify(q.recipe),
    cost_cif: q.cost_cif,
    markup: q.markup,
    quoted_price: q.quoted_price,
    total_tons: q.total_tons ?? null,
    contract_name: q.contract_name ?? null,
    status: q.status,
    note: q.note ?? null,
  };
}

export async function apiListQuotes(): Promise<Quote[]> {
  const resp = await authFetch("/api/quotes");
  if (!resp.ok) throw new Error(`GET /api/quotes 失败: ${resp.status}`);
  const data = (await resp.json()) as { quotes: QuoteWire[] };
  return data.quotes.map(wireToQuote);
}

export async function apiUpsertQuote(q: Quote): Promise<void> {
  const resp = await authFetch("/api/quotes", {
    method: "POST",
    body: JSON.stringify(quoteToWire(q)),
  });
  if (!resp.ok) {
    const e = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(e.error || `POST /api/quotes 失败: ${resp.status}`);
  }
}

export async function apiDeleteQuote(id: string): Promise<void> {
  const resp = await authFetch(
    `/api/quotes?id=${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/quotes 失败: ${resp.status}`);
}

// ============================================================
// Contracts
// ============================================================

interface ContractWire {
  id: string;
  quote_id: string | null;
  customer_id: string;
  customer_name: string;
  contract_no: string | null;
  billing_location: string | null;
  prepay_party: string | null;
  recipe_json: string;
  unit_price: number;
  total_tons: number;
  total_amount: number;
  first_pay_pct: number;
  first_pay_amount: number;
  tail_pay_amount: number;
  signed_at: string | null;
  status: string;
  note: string | null;
  created_at?: string;
  updated_at?: string;
}

function wireToContract(w: ContractWire): Contract {
  let recipe: Record<string, number> = {};
  try {
    recipe = JSON.parse(w.recipe_json) as Record<string, number>;
  } catch {
    /* ignore */
  }
  return {
    id: w.id,
    quote_id: w.quote_id,
    customer_id: w.customer_id,
    customer_name: w.customer_name,
    contract_no: w.contract_no,
    billing_location: w.billing_location,
    prepay_party: w.prepay_party,
    recipe,
    unit_price: w.unit_price,
    total_tons: w.total_tons,
    total_amount: w.total_amount,
    first_pay_pct: w.first_pay_pct,
    first_pay_amount: w.first_pay_amount,
    tail_pay_amount: w.tail_pay_amount,
    signed_at: w.signed_at,
    status: w.status as Contract["status"],
    note: w.note,
    created_at: w.created_at,
    updated_at: w.updated_at,
  };
}

function contractToWire(c: Contract): ContractWire {
  return {
    id: c.id,
    quote_id: c.quote_id ?? null,
    customer_id: c.customer_id,
    customer_name: c.customer_name,
    contract_no: c.contract_no ?? null,
    billing_location: c.billing_location ?? null,
    prepay_party: c.prepay_party ?? null,
    recipe_json: JSON.stringify(c.recipe),
    unit_price: c.unit_price,
    total_tons: c.total_tons,
    total_amount: c.total_amount,
    first_pay_pct: c.first_pay_pct,
    first_pay_amount: c.first_pay_amount,
    tail_pay_amount: c.tail_pay_amount,
    signed_at: c.signed_at ?? null,
    status: c.status,
    note: c.note ?? null,
  };
}

export async function apiListContracts(): Promise<Contract[]> {
  const resp = await authFetch("/api/contracts");
  if (!resp.ok) throw new Error(`GET /api/contracts 失败: ${resp.status}`);
  const data = (await resp.json()) as { contracts: ContractWire[] };
  return data.contracts.map(wireToContract);
}

export async function apiUpsertContract(c: Contract): Promise<void> {
  const resp = await authFetch("/api/contracts", {
    method: "POST",
    body: JSON.stringify(contractToWire(c)),
  });
  if (!resp.ok) {
    const e = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(e.error || `POST /api/contracts 失败: ${resp.status}`);
  }
}

export async function apiDeleteContract(id: string): Promise<void> {
  const resp = await authFetch(
    `/api/contracts?id=${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/contracts 失败: ${resp.status}`);
}

// ============================================================
// Payments
// ============================================================

export async function apiListPayments(contractId?: string): Promise<Payment[]> {
  const path = contractId
    ? `/api/payments?contract_id=${encodeURIComponent(contractId)}`
    : "/api/payments";
  const resp = await authFetch(path);
  if (!resp.ok) throw new Error(`GET /api/payments 失败: ${resp.status}`);
  const data = (await resp.json()) as { payments: Payment[] };
  return data.payments.map((p) => ({ ...p, kind: p.kind as Payment["kind"] }));
}

export async function apiUpsertPayment(p: Payment): Promise<void> {
  const resp = await authFetch("/api/payments", {
    method: "POST",
    body: JSON.stringify(p),
  });
  if (!resp.ok) {
    const e = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(e.error || `POST /api/payments 失败: ${resp.status}`);
  }
}

export async function apiDeletePayment(id: string): Promise<void> {
  const resp = await authFetch(
    `/api/payments?id=${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/payments 失败: ${resp.status}`);
}

// ============================================================
// Shipments
// ============================================================

interface ShipmentWire extends Omit<Shipment, "assay"> {
  assay_json: string | null;
}

function wireToShipment(w: ShipmentWire): Shipment {
  let assay: Partial<Record<string, number>> | null = null;
  if (w.assay_json) {
    try {
      assay = JSON.parse(w.assay_json) as Partial<Record<string, number>>;
    } catch {
      assay = null;
    }
  }
  const { assay_json: _drop, ...rest } = w;
  void _drop;
  return { ...rest, status: w.status as Shipment["status"], assay };
}

function shipmentToWire(s: Shipment): ShipmentWire {
  const { assay, ...rest } = s;
  return {
    ...rest,
    assay_json: assay ? JSON.stringify(assay) : null,
  };
}

export async function apiListShipments(contractId?: string): Promise<Shipment[]> {
  const path = contractId
    ? `/api/shipments?contract_id=${encodeURIComponent(contractId)}`
    : "/api/shipments";
  const resp = await authFetch(path);
  if (!resp.ok) throw new Error(`GET /api/shipments 失败: ${resp.status}`);
  const data = (await resp.json()) as { shipments: ShipmentWire[] };
  return data.shipments.map(wireToShipment);
}

export async function apiUpsertShipment(s: Shipment): Promise<void> {
  const resp = await authFetch("/api/shipments", {
    method: "POST",
    body: JSON.stringify(shipmentToWire(s)),
  });
  if (!resp.ok) {
    const e = (await resp.json().catch(() => ({}))) as { error?: string };
    throw new Error(e.error || `POST /api/shipments 失败: ${resp.status}`);
  }
}

export async function apiDeleteShipment(id: string): Promise<void> {
  const resp = await authFetch(
    `/api/shipments?id=${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!resp.ok) throw new Error(`DELETE /api/shipments 失败: ${resp.status}`);
}
