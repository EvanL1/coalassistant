/**
 * 合同详情 - Phase 2 的核心闭环.
 * 显示:
 *   - 合同基本信息 + 状态控制
 *   - 收款进度 (已收 / 应收, 进度条)
 *   - 发货进度 (已发 / 合同, 进度条)
 *   - 收款列表 + 加一笔
 *   - 发货列表 + 加一笔
 *
 * 不在 storage cache 里, 进来时现拉 payments + shipments.
 */
import { useEffect, useMemo, useState } from "react";
import type {
  Contract,
  ContractStatus,
  Payment,
  Shipment,
} from "./types";
import {
  apiDeletePayment,
  apiDeleteShipment,
  apiListPayments,
  apiListShipments,
} from "./api";
import { removeContract, upsertContract } from "./storage";
import { PaymentDialog } from "./PaymentDialog";
import { ShipmentDialog } from "./ShipmentDialog";

interface Props {
  contract: Contract;
  onClose: () => void;
}

const STATUS_LABEL: Record<ContractStatus, string> = {
  active: "执行中",
  completed: "已完结",
  terminated: "已终止",
};

const STATUS_COLOR: Record<ContractStatus, string> = {
  active: "var(--c-primary)",
  completed: "#10b981",
  terminated: "var(--c-danger)",
};

function fmtMoney(n: number): string {
  return n.toLocaleString("zh-CN", { maximumFractionDigits: 0 });
}

function fmtDate(iso?: string | null): string {
  if (!iso) return "";
  return iso.slice(0, 10);
}

export function ContractDetailDialog({ contract, onClose }: Props) {
  const [payments, setPayments] = useState<Payment[]>([]);
  const [shipments, setShipments] = useState<Shipment[]>([]);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState<ContractStatus>(contract.status);
  const [showAddPayment, setShowAddPayment] = useState(false);
  const [showAddShipment, setShowAddShipment] = useState(false);
  const [editPayment, setEditPayment] = useState<Payment | null>(null);
  const [editShipment, setEditShipment] = useState<Shipment | null>(null);

  useEffect(() => {
    const orig = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = orig;
    };
  }, []);

  async function reload() {
    setLoading(true);
    try {
      const [p, s] = await Promise.all([
        apiListPayments(contract.id),
        apiListShipments(contract.id),
      ]);
      setPayments(p);
      setShipments(s);
    } catch (e) {
      console.warn("reload contract detail 失败:", e);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void reload();
  }, [contract.id]);

  const totalPaid = useMemo(
    () => payments.reduce((sum, p) => sum + p.amount, 0),
    [payments],
  );
  const totalShipped = useMemo(
    () => shipments.reduce((sum, s) => sum + s.net_tons, 0),
    [shipments],
  );

  const payPct = contract.total_amount
    ? Math.min(100, (totalPaid / contract.total_amount) * 100)
    : 0;
  const shipPct = contract.total_tons
    ? Math.min(100, (totalShipped / contract.total_tons) * 100)
    : 0;

  async function changeStatus(newStatus: ContractStatus) {
    if (newStatus === status) return;
    if (
      newStatus === "terminated" &&
      !confirm("终止合同? 已记录的收款/发货不会删除.")
    )
      return;
    try {
      await upsertContract({ ...contract, status: newStatus });
      setStatus(newStatus);
    } catch (e) {
      alert(`改状态失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  async function deletePayment(p: Payment) {
    if (!confirm("删除该收款记录?")) return;
    try {
      await apiDeletePayment(p.id);
      await reload();
    } catch (e) {
      alert(`删除失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  async function deleteShipment(s: Shipment) {
    if (!confirm("删除该发货记录?")) return;
    try {
      await apiDeleteShipment(s.id);
      await reload();
    } catch (e) {
      alert(`删除失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  async function deleteContract() {
    if (!confirm("删除整个合同? 关联的收款/发货也会一并删除, 不可恢复.")) return;
    try {
      await removeContract(contract.id);
      onClose();
    } catch (e) {
      alert(`删除失败: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  return (
    <>
      <div className="modal-backdrop" onClick={onClose}>
        <div className="modal-sheet" onClick={(e) => e.stopPropagation()}>
          <div className="modal-handle" />
          <div className="modal-header">
            <div>
              <div className="modal-title">{contract.customer_name}</div>
              <div className="modal-subtitle">
                {contract.contract_no || "无合同号"} · {fmtDate(contract.signed_at)}
                {" · "}
                <span style={{ color: STATUS_COLOR[status] }}>
                  {STATUS_LABEL[status]}
                </span>
              </div>
            </div>
            <button className="modal-close" onClick={onClose}>×</button>
          </div>

          <div className="modal-body">
            {/* 关键数字 */}
            <div className="edit-section">
              <div className="edit-row">
                <div className="edit-row-label">单价</div>
                <div style={{ fontWeight: 600 }}>
                  ¥{contract.unit_price.toFixed(2)} /吨
                </div>
              </div>
              <div className="edit-row">
                <div className="edit-row-label">总吨数</div>
                <div style={{ fontWeight: 600 }}>{fmtMoney(contract.total_tons)} 吨</div>
              </div>
              <div className="edit-row" style={{ background: "var(--c-bg)" }}>
                <div className="edit-row-label">合同总额</div>
                <div style={{ fontWeight: 700, color: "var(--c-primary)" }}>
                  ¥{fmtMoney(contract.total_amount)}
                </div>
              </div>
              {contract.billing_location && (
                <div className="edit-row">
                  <div className="edit-row-label">开票地</div>
                  <div>{contract.billing_location}</div>
                </div>
              )}
              {contract.prepay_party && (
                <div className="edit-row">
                  <div className="edit-row-label">垫资方</div>
                  <div>{contract.prepay_party}</div>
                </div>
              )}
            </div>

            {/* 收款进度 */}
            <div className="edit-section">
              <div
                className="edit-section-title"
                style={{ display: "flex", justifyContent: "space-between" }}
              >
                <span>收款</span>
                <span style={{ fontSize: 11, color: "var(--c-text-3)", fontWeight: 400 }}>
                  ¥{fmtMoney(totalPaid)} / ¥{fmtMoney(contract.total_amount)}
                </span>
              </div>
              <ProgressBar pct={payPct} color="#10b981" />
              <div style={{ fontSize: 11, color: "var(--c-text-3)", marginTop: 4 }}>
                首付应收 ¥{fmtMoney(contract.first_pay_amount)} · 尾款应收 ¥
                {fmtMoney(contract.tail_pay_amount)} · 差额 ¥
                {fmtMoney(contract.total_amount - totalPaid)}
              </div>

              {payments.length === 0 ? (
                <div
                  style={{
                    fontSize: 12,
                    color: "var(--c-text-3)",
                    padding: "12px 0",
                  }}
                >
                  {loading ? "加载中..." : "还没收款记录"}
                </div>
              ) : (
                <div style={{ marginTop: 8 }}>
                  {payments.map((p) => (
                    <div
                      key={p.id}
                      onClick={() => setEditPayment(p)}
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        padding: "8px 0",
                        borderBottom: "1px solid var(--c-border)",
                        cursor: "pointer",
                      }}
                    >
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 13, fontWeight: 500 }}>
                          {p.kind === "first" ? "首付" :
                           p.kind === "tail" ? "尾款" :
                           p.kind === "advance" ? "预付" : "其他"}
                          {p.method && (
                            <span style={{ fontSize: 11, color: "var(--c-text-3)", marginLeft: 6, fontWeight: 400 }}>
                              {p.method}
                            </span>
                          )}
                        </div>
                        <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
                          {fmtDate(p.paid_at)}
                          {p.payer && ` · ${p.payer}`}
                        </div>
                      </div>
                      <div style={{ fontSize: 14, fontWeight: 600, color: "#10b981" }}>
                        +¥{fmtMoney(p.amount)}
                      </div>
                      <button
                        onClick={(e) => { e.stopPropagation(); void deletePayment(p); }}
                        style={{ marginLeft: 8, fontSize: 11, color: "var(--c-text-3)", padding: "2px 6px" }}
                      >
                        ×
                      </button>
                    </div>
                  ))}
                </div>
              )}

              <button
                className="btn btn-secondary"
                style={{ width: "100%", marginTop: 8 }}
                onClick={() => setShowAddPayment(true)}
              >
                + 记一笔收款
              </button>
            </div>

            {/* 发货进度 */}
            <div className="edit-section">
              <div
                className="edit-section-title"
                style={{ display: "flex", justifyContent: "space-between" }}
              >
                <span>发货</span>
                <span style={{ fontSize: 11, color: "var(--c-text-3)", fontWeight: 400 }}>
                  {fmtMoney(totalShipped)} / {fmtMoney(contract.total_tons)} 吨
                </span>
              </div>
              <ProgressBar pct={shipPct} color="var(--c-primary)" />

              {shipments.length === 0 ? (
                <div
                  style={{
                    fontSize: 12,
                    color: "var(--c-text-3)",
                    padding: "12px 0",
                  }}
                >
                  {loading ? "加载中..." : "还没发货记录"}
                </div>
              ) : (
                <div style={{ marginTop: 8 }}>
                  {shipments.map((s) => (
                    <div
                      key={s.id}
                      onClick={() => setEditShipment(s)}
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        alignItems: "center",
                        padding: "8px 0",
                        borderBottom: "1px solid var(--c-border)",
                        cursor: "pointer",
                      }}
                    >
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 13, fontWeight: 500 }}>
                          {s.vehicle_no || "未填车号"}
                          <span style={{ fontSize: 10, color: "var(--c-text-3)", marginLeft: 6, fontWeight: 400 }}>
                            {s.status === "shipped" ? "已发" :
                             s.status === "arrived" ? "到货" : "结算"}
                          </span>
                        </div>
                        <div style={{ fontSize: 11, color: "var(--c-text-3)" }}>
                          {fmtDate(s.shipped_at)}
                        </div>
                      </div>
                      <div style={{ fontSize: 14, fontWeight: 600, color: "var(--c-primary)" }}>
                        {fmtMoney(s.net_tons)} 吨
                      </div>
                      <button
                        onClick={(e) => { e.stopPropagation(); void deleteShipment(s); }}
                        style={{ marginLeft: 8, fontSize: 11, color: "var(--c-text-3)", padding: "2px 6px" }}
                      >
                        ×
                      </button>
                    </div>
                  ))}
                </div>
              )}

              <button
                className="btn btn-secondary"
                style={{ width: "100%", marginTop: 8 }}
                onClick={() => setShowAddShipment(true)}
              >
                + 记一笔发货
              </button>
            </div>

            {/* 状态 */}
            <div className="edit-section">
              <div className="edit-section-title">合同状态</div>
              <div style={{ display: "flex", gap: 6 }}>
                {(["active", "completed", "terminated"] as ContractStatus[]).map((s) => (
                  <button
                    key={s}
                    onClick={() => void changeStatus(s)}
                    style={{
                      padding: "6px 14px",
                      borderRadius: 999,
                      fontSize: 13,
                      background: status === s ? STATUS_COLOR[s] : "var(--c-bg)",
                      color: status === s ? "white" : "var(--c-text-2)",
                      fontWeight: status === s ? 600 : 400,
                    }}
                  >
                    {STATUS_LABEL[s]}
                  </button>
                ))}
              </div>
            </div>

            {contract.note && (
              <div className="edit-section">
                <div className="edit-section-title">备注</div>
                <p style={{ margin: 0, fontSize: 13, color: "var(--c-text-2)" }}>
                  {contract.note}
                </p>
              </div>
            )}

            <button
              className="btn btn-secondary"
              style={{ width: "100%", marginTop: 12, color: "var(--c-danger)" }}
              onClick={deleteContract}
            >
              删除合同
            </button>
          </div>

          <div className="modal-footer">
            <button className="btn btn-primary" style={{ width: "100%" }} onClick={onClose}>
              关闭
            </button>
          </div>
        </div>
      </div>

      {showAddPayment && (
        <PaymentDialog
          contractId={contract.id}
          defaultKind={payments.some((p) => p.kind === "first") ? "tail" : "first"}
          defaultAmount={
            payments.some((p) => p.kind === "first")
              ? contract.tail_pay_amount
              : contract.first_pay_amount
          }
          onClose={() => setShowAddPayment(false)}
          onSaved={() => void reload()}
        />
      )}
      {editPayment && (
        <PaymentDialog
          contractId={contract.id}
          editing={editPayment}
          onClose={() => setEditPayment(null)}
          onSaved={() => void reload()}
        />
      )}
      {showAddShipment && (
        <ShipmentDialog
          contractId={contract.id}
          onClose={() => setShowAddShipment(false)}
          onSaved={() => void reload()}
        />
      )}
      {editShipment && (
        <ShipmentDialog
          contractId={contract.id}
          editing={editShipment}
          onClose={() => setEditShipment(null)}
          onSaved={() => void reload()}
        />
      )}
    </>
  );
}

function ProgressBar({ pct, color }: { pct: number; color: string }) {
  return (
    <div
      style={{
        height: 8,
        background: "var(--c-bg)",
        borderRadius: 999,
        overflow: "hidden",
        marginTop: 6,
      }}
    >
      <div
        style={{
          width: `${pct}%`,
          height: "100%",
          background: color,
          transition: "width 0.3s",
        }}
      />
    </div>
  );
}
