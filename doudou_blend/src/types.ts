/** 跟 blend_kit_rs schema 对齐的 TypeScript 类型. */

export type IndicatorKey = "S" | "A" | "V" | "G" | "Y" | "petro" | "CSR" | "M";

export const INDICATOR_LABEL: Record<string, string> = {
  S: "硫",
  A: "灰",
  V: "挥发",
  G: "粘结",
  Y: "胶质",
  petro: "岩相",
  CSR: "焦炭强度",
  M: "水分",
};

export const INDICATOR_ORDER: IndicatorKey[] = [
  "S",
  "A",
  "V",
  "G",
  "Y",
  "petro",
  "CSR",
  "M",
];

export type Direction = "Upper" | "Lower" | "Range";

export interface Spec {
  indicator: string;
  direction: Direction;
  min?: number | null;
  max?: number | null;
  enabled?: boolean;
}

export interface Coal {
  name: string;
  props: Partial<Record<string, number>>;
  fob: number;
  frt: number;
}

export interface BlendRequest {
  coals: Coal[];
  specs: Spec[];
  total_quantity?: number | null;
  truncate_decimal?: boolean;
}

export interface CostBreakdown {
  fob_per_ton: number;
  frt_per_ton: number;
  cif_per_ton: number;
  total_fob?: number | null;
  total_frt?: number | null;
  total_cif?: number | null;
}

export interface OrderItem {
  coal: string;
  ratio: number;
  tons?: number | null;
  fob_amount?: number | null;
  frt_amount?: number | null;
  cif_amount?: number | null;
}

export interface IndicatorCheck {
  indicator: string;
  label_zh: string;
  value: number;
  min?: number | null;
  max?: number | null;
  slack?: number | null;
  binding: boolean;
}

export interface BlendResult {
  ok: boolean;
  reason?: string | null;
  recipe: Record<string, number>;
  cost?: CostBreakdown | null;
  orders: OrderItem[];
  indicator_check: IndicatorCheck[];
  warnings: string[];
}

// ===== Master schema =====

export type CoalStatus = "verified" | "active" | "draft" | "incomplete" | "archived";
export type Confidence = "high" | "medium" | "low";

export interface MasterCoalEntry {
  name: string;
  region?: string | null;
  coal_type?: string | null;
  status: CoalStatus;
  props: Partial<Record<string, number>>;
  fob?: number | null;
  frt?: number | null;
  confidence?: Record<string, Confidence>;
  note?: string | null;
}

export interface DefaultContract {
  name: string;
  specs: Spec[];
}

export interface CoalMaster {
  version: string;
  updated_at: string;
  description: string;
  default_contract: DefaultContract;
  coals: MasterCoalEntry[];
}

// ===== Phase 1: Pre 合同流程 =====

export interface Customer {
  id: string;
  name: string;
  contact?: string | null;
  phone?: string | null;
  note?: string | null;
  created_at?: string;
  updated_at?: string;
}

/** 报价单状态: 草稿 / 已发 / 已签 / 已弃 */
export type QuoteStatus = "draft" | "sent" | "signed" | "lost";

export interface Quote {
  id: string;
  customer_id: string;
  customer_name: string;       // 冗余存, 客户改名后报价历史仍能展示原名
  recipe: Record<string, number>; // 煤名 → 比例 (sum ≈ 1.0)
  cost_cif: number;            // 算出的成本 元/吨
  markup: number;              // 利润加成 元/吨
  quoted_price: number;        // = cost_cif + markup
  total_tons?: number | null;
  contract_name?: string | null;
  status: QuoteStatus;
  note?: string | null;
  created_at?: string;
  updated_at?: string;
}
