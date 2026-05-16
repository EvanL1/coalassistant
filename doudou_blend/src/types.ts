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

// ===== Phase 2: 合同 / 收款 / 发货 =====

export type ContractStatus = "active" | "completed" | "terminated";

export interface Contract {
  id: string;
  quote_id?: string | null;
  customer_id: string;
  customer_name: string;
  contract_no?: string | null;
  billing_location?: string | null;  // 开票地, 如 "集宁"
  prepay_party?: string | null;      // 垫资方, "self" / 别的公司名
  recipe: Record<string, number>;    // 锁定的配方
  unit_price: number;                // 单价 元/吨
  total_tons: number;                // 合同总吨数
  total_amount: number;              // 合同总额 = unit_price * total_tons
  first_pay_pct: number;             // 首付比例, 默认 80
  first_pay_amount: number;          // 首付金额
  tail_pay_amount: number;           // 尾款金额
  signed_at?: string | null;         // 签约日
  status: ContractStatus;
  note?: string | null;
  created_at?: string;
  updated_at?: string;
}

export type PaymentKind = "first" | "tail" | "advance" | "other";

export interface Payment {
  id: string;
  contract_id: string;
  kind: PaymentKind;
  amount: number;
  paid_at: string;          // 收款日
  payer?: string | null;    // 实际打款方
  method?: string | null;   // "打款" / "现金" / "票据"
  voucher_no?: string | null;
  note?: string | null;
  created_at?: string;
}

export type ShipmentStatus = "shipped" | "arrived" | "settled";

export interface Shipment {
  id: string;
  contract_id: string;
  vehicle_no?: string | null;
  net_tons: number;
  gross_tons?: number | null;
  tare_tons?: number | null;
  shipped_at: string;       // 发货日
  arrived_at?: string | null;
  settled_at?: string | null;
  settled_amount?: number | null;
  assay?: Partial<Record<string, number>> | null; // 化验值, 8 项指标
  status: ShipmentStatus;
  note?: string | null;
  created_at?: string;
}
